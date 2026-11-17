import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { Link, Outlet, useNavigate, useRouter, useRouterState } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Check,
  ChevronDown,
  ChevronRight,
  FileText,
  Folder,
  FolderOpen,
  Lock,
  MoreHorizontal,
  PanelLeft,
  Trash2,
} from 'lucide-react'
import type { NoteTreeNode } from '~/lib/vaultApi'
import { noteTreeQuery } from '~/lib/queries'
import { usePersistedState } from '~/lib/usePersistedState'
import { Sheet } from '~/components/ui/Sheet'
import { t } from '~/strings'
import {
  ancestorPaths,
  findNode,
  foldTrail,
  furthestIndex,
  looksLikeFile,
  noteName,
  siblingsOf,
  trailOf,
  treeMove,
  treeRows,
  upTarget,
  vaultLocation,
  VAULT_NAME,
  type HistoryAction,
  type SiblingItem,
  type TrailPart,
  type TreeKey,
  type TreeRow,
} from './noteRows'
import './notes.css'

const NOTEBOOKS = 'Notebooks'
/** The tree opens by itself from this width, in a folder or a note. */
const WIDE_FROM = 1280
/** From this width the tree is a panel beside the page; below it, a sheet. */
const PANEL_FROM = 1024
/** How many opened folders are remembered on this device. */
const REMEMBER = 200
/** A crumb menu with more folders than this gets a filter field. */
const FILTER_FROM = 8

type PathSet = ReadonlySet<string>

/** One part of the location bar as a link to the folder or note it opens; Vault home is `/notes`. */
function PartLink({ part, className, onClick }: { part: TrailPart; className?: string; onClick?: () => void }) {
  if (part.path === '') {
    return (
      <Link to="/notes" className={className} onClick={onClick}>
        {part.name}
      </Link>
    )
  }
  return (
    <Link to="/notes/$" params={{ _splat: part.path }} className={className} onClick={onClick}>
      {part.name}
    </Link>
  )
}

/** A menu that opens under a button, in fixed position so the bar's own clipping cannot hide it. */
function useFloatingMenu() {
  const [at, setAt] = useState<{ left: number; top: number } | null>(null)
  const root = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const open = at !== null
  const close = useCallback(() => setAt(null), [])
  useEffect(() => {
    if (!open) return
    const outside = (e: Event) => {
      if (root.current?.contains(e.target as Node)) return
      setAt(null)
    }
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== 'Escape') return
      setAt(null)
      trigger.current?.focus()
    }
    document.addEventListener('pointerdown', outside)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('pointerdown', outside)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])
  const toggle = () => {
    if (open) return setAt(null)
    const r = trigger.current?.getBoundingClientRect()
    setAt(r ? { left: r.left, top: r.bottom + 4 } : { left: 0, top: 0 })
  }
  return { at, open, close, toggle, root, trigger }
}

/** Arrow keys move between the items of a menu, and Home and End jump to its ends. */
function menuKeys(e: KeyboardEvent<HTMLElement>) {
  if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) return
  const items = Array.from(e.currentTarget.querySelectorAll('[role="menuitem"],[role="menuitemcheckbox"]')) as HTMLElement[]
  if (items.length === 0) return
  const at = items.indexOf(document.activeElement as HTMLElement)
  const last = items.length - 1
  let next = 0
  if (e.key === 'End') next = last
  else if (e.key === 'ArrowDown') next = at < last ? at + 1 : 0
  else if (e.key === 'ArrowUp') next = at > 0 ? at - 1 : last
  e.preventDefault()
  items[next].focus()
}

/** The middle of a long trail, folded into one menu. */
function TrailMore({ hidden }: { hidden: TrailPart[] }) {
  const menu = useFloatingMenu()
  return (
    <div ref={menu.root} className="vn-more">
      <button ref={menu.trigger} type="button" className="vn-crumb vn-crumb-btn" aria-haspopup="menu" aria-expanded={menu.open} aria-label={t('vault_trail_more')} onClick={menu.toggle}>
        <MoreHorizontal className="a-ic xs" aria-hidden="true" />
      </button>
      {menu.at && (
        <div className="a-menu vn-menu" role="menu" aria-label={t('vault_trail_more')} style={menu.at} onKeyDown={menuKeys}>
          {hidden.map((part) => (
            <PartLink key={part.path} part={part} className="a-menu-item" onClick={menu.close} />
          ))}
        </div>
      )}
    </div>
  )
}

function SiblingLink({ item, current, onPick }: { item: SiblingItem; current: boolean; onPick: () => void }) {
  return (
    <Link to="/notes/$" params={{ _splat: item.path }} role="menuitem" aria-current={current ? 'page' : undefined} className="a-menu-item vn-sibling" onClick={onPick}>
      <span className="vn-sibling-mark" aria-hidden="true">
        {current && <Check className="a-ic xs" />}
      </span>
      <span className="vn-sibling-name">{item.name}</span>
      {item.isDir && <span className="vn-count">{item.count}</span>}
    </Link>
  )
}

/** The chevron on a crumb: the folders (or notes) beside it, with a filter when there are many. */
function CrumbMenu({ part, root, isFile }: { part: TrailPart; root: NoteTreeNode | undefined; isFile: boolean }) {
  const menu = useFloatingMenu()
  const [filter, setFilter] = useState('')
  const { parent, items } = useMemo(() => siblingsOf(root, part.path), [root, part.path])
  if (items.length < 2) return null
  const folder = parent && parent !== NOTEBOOKS ? noteName(parent) : VAULT_NAME
  const shown = filter ? items.filter((i) => i.name.toLowerCase().includes(filter.toLowerCase())) : items
  const label = t(isFile ? 'vault_beside_notes' : 'vault_beside', { name: part.name })
  const openLabel = t('vault_open_folder', { name: folder })
  const filterable = FILTER_FROM < items.length
  return (
    <div ref={menu.root} className="vn-more vn-crumb-more">
      <button
        ref={menu.trigger}
        type="button"
        className="vn-chev-btn"
        aria-haspopup="menu"
        aria-expanded={menu.open}
        aria-label={label}
        onClick={() => {
          setFilter('')
          menu.toggle()
        }}
      >
        <ChevronDown className="a-ic xs" aria-hidden="true" />
      </button>
      {menu.at && (
        <div className="a-menu vn-menu" role="menu" aria-label={label} style={menu.at} onKeyDown={menuKeys}>
          <p className="a-label vn-menu-head">{t('vault_siblings_in', { folder })}</p>
          {filterable && (
            <input type="search" className="a-input vn-filter" aria-label={t('vault_filter')} placeholder={t('vault_filter')} value={filter} onChange={(e) => setFilter(e.target.value)} />
          )}
          {shown.map((item) => (
            <SiblingLink key={item.path} item={item} current={item.path === part.path} onPick={menu.close} />
          ))}
          {parent && parent !== NOTEBOOKS && <PartLink part={{ name: openLabel, path: parent }} className="a-menu-item vn-menu-open" onClick={menu.close} />}
        </div>
      )}
    </div>
  )
}

/** Up: the folder above, Vault home from a top-level folder, and dimmed on Vault home. */
function UpLink({ up }: { up: string | null }) {
  const name = up === null || up === '' ? VAULT_NAME : noteName(up)
  const label = t('vault_up', { name })
  const inner = (
    <>
      <ArrowUp className="a-ic sm" aria-hidden="true" />
      <span className="vn-up-label">{name}</span>
    </>
  )
  if (up === null) {
    return (
      <span className="a-ib vn-up is-off" aria-disabled="true" aria-label={label}>
        {inner}
      </span>
    )
  }
  if (up === '') {
    return (
      <Link to="/notes" className="a-ib vn-up" aria-label={label} title={`${label} (Alt ↑)`}>
        {inner}
      </Link>
    )
  }
  return (
    <Link to="/notes/$" params={{ _splat: up }} className="a-ib vn-up" aria-label={label} title={`${label} (Alt ↑)`}>
      {inner}
    </Link>
  )
}

interface HistoryButtonsProps {
  canBack: boolean
  canForward: boolean
  onBack: () => void
  onForward: () => void
}

/** Back and Forward follow the history, as a browser's do, and are dimmed when there is nowhere to go. */
function HistoryButtons({ canBack, canForward, onBack, onForward }: HistoryButtonsProps) {
  return (
    <div className="vn-hist">
      <button type="button" className="a-ib sm vn-back" disabled={!canBack} aria-label={t('vault_back')} title={`${t('vault_back')} (Alt ←)`} onClick={onBack}>
        <ArrowLeft className="a-ic sm" aria-hidden="true" />
      </button>
      <button type="button" className="a-ib sm vn-forward" disabled={!canForward} aria-label={t('vault_forward')} title={`${t('vault_forward')} (Alt →)`} onClick={onForward}>
        <ArrowRight className="a-ic sm" aria-hidden="true" />
      </button>
    </div>
  )
}

export interface LocationBarProps {
  /** The Vault path (a folder or a note); empty on Vault home. */
  path: string
  isFile: boolean
  /** The Trash and other pages that sit under Vault: the last part is plain text. */
  extra?: string
  /** The folder tree, for the menus on the crumbs. */
  root?: NoteTreeNode
  treeOpen: boolean
  /** Whether the Folders button opens a panel beside the page or a sheet. */
  treeMode: 'panel' | 'sheet'
  onToggleTree: () => void
  canBack?: boolean
  canForward?: boolean
  onBack?: () => void
  onForward?: () => void
}

const noop = () => {}

/** Where you are in the Vault, with Back, Forward and Up, and the button that opens the folder tree. */
export function LocationBar({ path, isFile, extra, root, treeOpen, treeMode, onToggleTree, canBack = false, canForward = false, onBack = noop, onForward = noop }: LocationBarProps) {
  const trail = trailOf(path, isFile)
  const parts = extra ? [...trail, { name: extra, path: '' }] : trail
  const up = extra ? '' : upTarget(path)
  const { before, hidden, after } = foldTrail(parts)
  const last = parts.length - 1
  const render = (part: TrailPart, index: number) => {
    const current = index === last
    return (
      <li key={`${index}:${part.path}`} className={current ? 'is-current' : undefined}>
        {index > 0 && <ChevronRight className="a-ic xs vn-sep" aria-hidden="true" />}
        {current ? (
          <span className="vn-crumb vn-current" aria-current="page">
            {part.name}
          </span>
        ) : (
          <PartLink part={part} className="vn-crumb" />
        )}
        {part.path !== '' && !extra && <CrumbMenu part={part} root={root} isFile={current && isFile} />}
      </li>
    )
  }
  const sheet = treeMode === 'sheet'
  const toggleLabel = treeOpen && !sheet ? t('vault_tree_hide') : t('vault_tree_show')
  return (
    <div className="vn-bar">
      <HistoryButtons canBack={canBack} canForward={canForward} onBack={onBack} onForward={onForward} />
      <UpLink up={up} />
      <nav className="vn-crumbs" aria-label={t('vault_nav_label')}>
        <ol>
          {before.map((part, i) => render(part, i))}
          {hidden.length > 0 && (
            <li>
              <ChevronRight className="a-ic xs vn-sep" aria-hidden="true" />
              <TrailMore hidden={hidden} />
            </li>
          )}
          {after.map((part, i) => render(part, before.length + hidden.length + i))}
        </ol>
      </nav>
      <button
        type="button"
        className="a-ib vn-toggle"
        aria-pressed={sheet ? undefined : treeOpen}
        aria-haspopup={sheet ? 'dialog' : undefined}
        aria-controls={sheet ? undefined : 'vault-tree'}
        aria-label={toggleLabel}
        title={`${toggleLabel} (Ctrl B)`}
        onClick={onToggleTree}
      >
        <PanelLeft className="a-ic sm" aria-hidden="true" />
        <span className="vn-toggle-label">{t('notes_folders')}</span>
      </button>
    </div>
  )
}

export interface FolderTreeProps {
  root: NoteTreeNode | undefined
  loading: boolean
  failed: boolean
  /** The folder or note that is open, so its row is selected. */
  current: string
  open: PathSet
  showAll: PathSet
  showSystem: boolean
  /** A panel beside the page, or the body of a sheet. */
  variant?: 'panel' | 'sheet'
  onToggle: (path: string, open: boolean) => void
  onShowAll: (path: string) => void
  onShowSystem: (on: boolean) => void
  onCollapseAll: () => void
  onReveal: () => void
  /** Called when a row opens a page, so a sheet can close. */
  onNavigate?: () => void
}

type TreeMenuProps = Pick<FolderTreeProps, 'showSystem' | 'onShowSystem' | 'onCollapseAll' | 'onReveal'>

/** The "..." menu in the tree's head: collapse everything, find the open note, show system folders. */
function TreeMenu({ showSystem, onShowSystem, onCollapseAll, onReveal }: TreeMenuProps) {
  const menu = useFloatingMenu()
  const run = (fn: () => void) => () => {
    fn()
    menu.close()
  }
  return (
    <div ref={menu.root} className="vn-more">
      <button ref={menu.trigger} type="button" className="a-ib sm" aria-haspopup="menu" aria-expanded={menu.open} aria-label={t('vault_tree_menu')} onClick={menu.toggle}>
        <MoreHorizontal className="a-ic sm" aria-hidden="true" />
      </button>
      {menu.at && (
        <div className="a-menu vn-menu" role="menu" aria-label={t('vault_tree_menu')} style={menu.at} onKeyDown={menuKeys}>
          <button type="button" role="menuitem" className="a-menu-item" onClick={run(onCollapseAll)}>
            {t('vault_collapse_all')}
          </button>
          <button type="button" role="menuitem" className="a-menu-item" onClick={run(onReveal)}>
            {t('vault_reveal_open')}
          </button>
          <button type="button" role="menuitemcheckbox" aria-checked={showSystem} className="a-menu-item" onClick={run(() => onShowSystem(!showSystem))}>
            <span className="vn-sibling-mark" aria-hidden="true">
              {showSystem && <Check className="a-ic xs" />}
            </span>
            {t('vault_show_system')}
          </button>
        </div>
      )}
    </div>
  )
}

interface TreeRowViewProps {
  row: TreeRow
  tabStop: string | null
  current: string
  onPath: boolean
  onFocusRow: (path: string) => void
  onToggle: (path: string, open: boolean) => void
  onShowAll: (folder: string) => void
  onNavigate?: () => void
}

function RowChevron({ row, onToggle }: { row: TreeRow; onToggle: (path: string, open: boolean) => void }) {
  if (row.kind !== 'folder') {
    return <span className="vn-chev" />
  }
  return (
    <button type="button" tabIndex={-1} aria-hidden="true" className="vn-chev" onClick={() => onToggle(row.path, !row.open)}>
      {row.open ? <ChevronDown className="a-ic xs" /> : <ChevronRight className="a-ic xs" />}
    </button>
  )
}

function RowIcon({ row }: { row: TreeRow }) {
  if (row.kind === 'folder') {
    return row.open ? <FolderOpen className="a-ic xs" /> : <Folder className="a-ic xs" />
  }
  return <FileText className="a-ic xs" />
}

function TreeRowView({ row, tabStop, current, onPath, onFocusRow, onToggle, onShowAll, onNavigate }: TreeRowViewProps) {
  const indent = { paddingInlineStart: `${8 + row.depth * 16}px` }
  if (row.kind === 'more') {
    return (
      <div role="treeitem" aria-level={row.depth + 1} aria-selected={false} data-path={row.path} tabIndex={row.path === tabStop ? 0 : -1} className="vn-row vn-more-row" style={indent} onFocus={() => onFocusRow(row.path)}>
        <span className="vn-chev" />
        <button type="button" tabIndex={-1} className="vn-name vn-show-all" onClick={() => onShowAll(row.folder)}>
          {t('notes_see_all', { n: row.total })}
        </button>
      </div>
    )
  }
  const selected = row.path === current
  const system = row.kind === 'folder' && row.system
  const className = ['vn-row', selected ? 'is-on' : '', onPath && !selected ? 'is-path' : '', system ? 'is-system' : ''].filter(Boolean).join(' ')
  const label = row.kind === 'folder' ? t('vault_tree_count_label', { name: row.name, n: row.count }) : undefined
  return (
    <div
      role="treeitem"
      aria-level={row.depth + 1}
      aria-selected={selected}
      aria-expanded={row.kind === 'folder' ? row.open : undefined}
      aria-label={label}
      data-path={row.path}
      tabIndex={row.path === tabStop ? 0 : -1}
      className={className}
      style={indent}
      onFocus={() => onFocusRow(row.path)}
    >
      <RowChevron row={row} onToggle={onToggle} />
      <span className="vn-kind" aria-hidden="true">
        <RowIcon row={row} />
      </span>
      <Link to="/notes/$" params={{ _splat: row.path }} tabIndex={-1} className="vn-name" onClick={onNavigate}>
        {row.name}
      </Link>
      {system && <Lock className="a-ic xs vn-lock" aria-label={t('vault_system_note')} />}
      {row.kind === 'folder' && (
        <span className="vn-count" aria-hidden="true">
          {row.count}
        </span>
      )}
    </div>
  )
}

/** The folders of the Vault as a tree: one tab stop, arrows move, and the open path is bold. */
export function FolderTree({ root, loading, failed, current, open, showAll, showSystem, variant = 'panel', onToggle, onShowAll, onShowSystem, onCollapseAll, onReveal, onNavigate }: FolderTreeProps) {
  const navigate = useNavigate()
  const rows = useMemo(() => treeRows(root, open, showAll, undefined, showSystem), [root, open, showAll, showSystem])
  const ancestors = useMemo(() => new Set(current ? ancestorPaths(`${current}/x`) : []), [current])
  const [focusPath, setFocusPath] = useState<string | null>(null)
  const tree = useRef<HTMLDivElement>(null)

  // Tab lands on the open row, or on the first when none is open.
  const tabStop = (() => {
    if (focusPath && rows.some((r) => r.path === focusPath)) return focusPath
    return rows.find((r) => r.path === current)?.path ?? rows[0]?.path ?? null
  })()

  const find = useCallback((selector: string) => tree.current?.querySelector(selector) as HTMLElement | null | undefined, [])

  // Keep the open row in view when it changes, such as after a link from a search result.
  useEffect(() => {
    find('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' })
  }, [current, root, find])

  const focusRow = useCallback(
    (path: string) => {
      setFocusPath(path)
      requestAnimationFrame(() => find(`[data-path="${CSS.escape(path)}"]`)?.focus())
    },
    [find],
  )

  const activate = (row: TreeRow) => {
    if (row.kind === 'more') return onShowAll(row.folder)
    void navigate({ to: '/notes/$', params: { _splat: row.path } })
    onNavigate?.()
  }

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const index = rows.findIndex((r) => r.path === tabStop)
    if (index < 0) return
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      activate(rows[index])
      return
    }
    if (!['ArrowDown', 'ArrowUp', 'ArrowRight', 'ArrowLeft', 'Home', 'End'].includes(e.key)) return
    const move = treeMove(rows, index, e.key as TreeKey)
    e.preventDefault()
    if (!move) return
    if ('focus' in move) focusRow(rows[move.focus].path)
    else onToggle(move.toggle, move.open)
  }

  return (
    <aside id={variant === 'panel' ? 'vault-tree' : undefined} className={variant === 'sheet' ? 'vn-tree is-sheet' : 'vn-tree'} aria-label={t('notes_folders')}>
      <div className="vn-tree-head">
        <h2 className="a-label">{t('notes_folders')}</h2>
        <TreeMenu showSystem={showSystem} onShowSystem={onShowSystem} onCollapseAll={onCollapseAll} onReveal={onReveal} />
      </div>
      {loading && (
        <div role="status" className="vn-tree-body" aria-label={t('notes_viewer_loading')}>
          <span className="a-skel t" />
          <span className="a-skel t" />
          <span className="a-skel t" />
        </div>
      )}
      {failed && !root && <p className="a-body-sm a-muted vn-tree-note">{t('vault_tree_failed')}</p>}
      {root && (
        <div ref={tree} className="vn-tree-body" role="tree" aria-label={t('vault_tree_label')} onKeyDown={onKeyDown}>
          {rows.map((row) => (
            <TreeRowView key={row.path} row={row} tabStop={tabStop} current={current} onPath={ancestors.has(row.path)} onFocusRow={setFocusPath} onToggle={onToggle} onShowAll={onShowAll} onNavigate={onNavigate} />
          ))}
        </div>
      )}
      <div className="vn-tree-foot">
        <Link to="/notes/trash" className="vn-row vn-trash" onClick={onNavigate}>
          <span className="vn-chev" />
          <span className="vn-kind" aria-hidden="true">
            <Trash2 className="a-ic xs" />
          </span>
          <span className="vn-name">{t('notes_trash')}</span>
        </Link>
      </div>
    </aside>
  )
}

/** Whether the window is at least `px` wide, kept up to date. False before the first measurement. */
function useWindowAtLeast(px: number): boolean {
  const [wide, setWide] = useState(false)
  useEffect(() => {
    if (typeof window === 'undefined' || !window.matchMedia) return
    const query = window.matchMedia(`(min-width:${px}px)`)
    const update = () => setWide(query.matches)
    update()
    query.addEventListener('change', update)
    return () => query.removeEventListener('change', update)
  }, [px])
  return wide
}

/** Back, Forward and how far forward the history reaches, kept as the router's history changes. */
function useVaultHistory() {
  const router = useRouter()
  const read = () => (router.history.location.state as { __TSR_index?: number }).__TSR_index ?? 0
  const furthest = useRef(read())
  const [reach, setReach] = useState({ canBack: 0 < read(), canForward: false })
  useEffect(() => {
    return router.history.subscribe(({ location, action }) => {
      const index = (location.state as { __TSR_index?: number }).__TSR_index ?? 0
      furthest.current = furthestIndex(furthest.current, action.type as HistoryAction, index)
      setReach({ canBack: 0 < index, canForward: index < furthest.current })
    })
  }, [router])
  return { ...reach, back: () => router.history.back(), forward: () => router.history.forward() }
}

/** Whether a key event happens while the person is typing, where shortcuts must stay out of the way. */
function isTyping(e: globalThis.KeyboardEvent): boolean {
  const el = e.target as HTMLElement | null
  return !!el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName))
}

/** Every Vault page sits in this: the location bar on top, the folder tree beside the page or in a sheet. */
export function VaultShell({ children }: { children?: ReactNode }) {
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const navigate = useNavigate()
  const history = useVaultHistory()
  const { path, special } = vaultLocation(pathname)
  const treeQuery = useQuery(noteTreeQuery(NOTEBOOKS))
  const root = treeQuery.data?.tree
  const atHome = path === '' && special === null
  const wide = useWindowAtLeast(WIDE_FROM)
  const panelWidth = useWindowAtLeast(PANEL_FROM)
  const treeMode = panelWidth ? 'panel' : 'sheet'
  const [pref, setPref] = usePersistedState<boolean | null>('hq-vault-tree', null, (v) => v === null || typeof v === 'boolean')
  const [showSystem, setShowSystem] = usePersistedState<boolean>('hq-vault-tree-system', false, (v) => typeof v === 'boolean')
  const [openList, setOpenList] = usePersistedState<string[]>('hq-vault-tree-open', [], Array.isArray)
  const [showAll, setShowAll] = useState<PathSet>(new Set())
  const [sheetOpen, setSheetOpen] = useState(false)
  const open = useMemo(() => new Set(openList), [openList])
  const treeOpen = pref ?? (wide && !atHome)

  const node = findNode(root, path)
  const isFile = node ? node.type === 'file' : looksLikeFile(path)

  const reveal = useCallback(() => {
    if (!path) return
    const need = ancestorPaths(isFile ? path : `${path}/x`).filter((p) => p !== NOTEBOOKS)
    setOpenList((prev) => (need.every((p) => prev.includes(p)) ? prev : [...prev, ...need.filter((p) => !prev.includes(p))].slice(-REMEMBER)))
  }, [path, isFile, setOpenList])
  // Whatever way the person arrived, the folders above the open item are open.
  useEffect(reveal, [reveal])
  // A sheet closes by itself once the person has gone somewhere.
  useEffect(() => setSheetOpen(false), [pathname])

  const toggleTree = useCallback(() => {
    if (treeMode === 'sheet') setSheetOpen((v) => !v)
    else setPref(!treeOpen)
  }, [treeMode, treeOpen, setPref])

  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (isTyping(e)) return
      const mod = e.ctrlKey || e.metaKey
      if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'b') {
        e.preventDefault()
        toggleTree()
        return
      }
      if (!e.altKey || mod || e.shiftKey) return
      if (e.key === 'ArrowLeft') {
        e.preventDefault()
        history.back()
      } else if (e.key === 'ArrowRight') {
        e.preventDefault()
        history.forward()
      } else if (e.key === 'ArrowUp') {
        const up = special ? '' : upTarget(path)
        if (up === null) return
        e.preventDefault()
        if (up === '') void navigate({ to: '/notes' })
        else void navigate({ to: '/notes/$', params: { _splat: up } })
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [toggleTree, history, navigate, path, special])

  const onToggle = (p: string, wantOpen: boolean) =>
    setOpenList((prev) => (wantOpen ? (prev.includes(p) ? prev : [...prev, p].slice(-REMEMBER)) : prev.filter((x) => x !== p)))

  const tree = (variant: 'panel' | 'sheet') => (
    <FolderTree
      root={root}
      loading={treeQuery.isPending}
      failed={treeQuery.isError}
      current={special === null ? path : ''}
      open={open}
      showAll={showAll}
      showSystem={showSystem}
      variant={variant}
      onToggle={onToggle}
      onShowAll={(p) => setShowAll((prev) => new Set(prev).add(p))}
      onShowSystem={setShowSystem}
      onCollapseAll={() => setOpenList([])}
      onReveal={reveal}
      onNavigate={variant === 'sheet' ? () => setSheetOpen(false) : undefined}
    />
  )

  return (
    <div className="vn-shell">
      <LocationBar
        path={path}
        isFile={isFile}
        extra={special === 'trash' ? t('notes_trash') : undefined}
        root={root}
        treeOpen={treeMode === 'sheet' ? sheetOpen : treeOpen}
        treeMode={treeMode}
        onToggleTree={toggleTree}
        canBack={history.canBack}
        canForward={history.canForward}
        onBack={history.back}
        onForward={history.forward}
      />
      <div className="vn-body">
        {treeMode === 'panel' && treeOpen && tree('panel')}
        <div className="vn-main">{children ?? <Outlet />}</div>
      </div>
      {treeMode === 'sheet' && sheetOpen && (
        <Sheet title={t('notes_folders')} placement="side" onClose={() => setSheetOpen(false)}>
          {tree('sheet')}
        </Sheet>
      )}
    </div>
  )
}
