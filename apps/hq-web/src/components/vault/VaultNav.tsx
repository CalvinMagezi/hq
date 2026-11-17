import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { Link, Outlet, useNavigate, useRouterState } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { ArrowUp, ChevronDown, ChevronRight, FileText, Folder, FolderOpen, MoreHorizontal, PanelLeft, Trash2 } from 'lucide-react'
import type { NoteTreeNode } from '~/lib/vaultApi'
import { noteTreeQuery } from '~/lib/queries'
import { usePersistedState } from '~/lib/usePersistedState'
import { t } from '~/strings'
import {
  ancestorPaths,
  findNode,
  foldTrail,
  looksLikeFile,
  noteName,
  trailOf,
  treeMove,
  treeRows,
  upTarget,
  vaultLocation,
  VAULT_NAME,
  type TrailPart,
  type TreeKey,
  type TreeRow,
} from './noteRows'
import './notes.css'

const NOTEBOOKS = 'Notebooks'
/** The tree opens by itself from this width, in a folder or a note. Below 1024 px it has no room. */
const WIDE_FROM = 1280
/** How many opened folders are remembered on this device. */
const REMEMBER = 200

type PathSet = ReadonlySet<string>

/** One part of the location bar as a link to the folder or note it opens; Vault home is `/notes`. */
function PartLink({ part, className, onClick }: { part: TrailPart; className?: string; onClick?: () => void }) {
  return part.path === '' ? (
    <Link to="/notes" className={className} onClick={onClick}>
      {part.name}
    </Link>
  ) : (
    <Link to="/notes/$" params={{ _splat: part.path }} className={className} onClick={onClick}>
      {part.name}
    </Link>
  )
}

/** The middle of a long trail, folded into one menu. Escape and a click outside close it. */
function TrailMore({ hidden }: { hidden: TrailPart[] }) {
  const [at, setAt] = useState<{ left: number; top: number } | null>(null)
  const root = useRef<HTMLDivElement>(null)
  const button = useRef<HTMLButtonElement>(null)
  const open = at !== null
  useEffect(() => {
    if (!open) return
    const close = (e: Event) => {
      if (root.current?.contains(e.target as Node)) return
      setAt(null)
    }
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== 'Escape') return
      setAt(null)
      button.current?.focus()
    }
    document.addEventListener('pointerdown', close)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('pointerdown', close)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])
  // The menu is placed from the button, in fixed position, so the bar's own clipping cannot hide it.
  const toggle = () => {
    if (open) return setAt(null)
    const r = button.current?.getBoundingClientRect()
    setAt(r ? { left: r.left, top: r.bottom + 4 } : { left: 0, top: 0 })
  }
  return (
    <div ref={root} className="vn-more">
      <button ref={button} type="button" className="vn-crumb vn-crumb-btn" aria-haspopup="menu" aria-expanded={open} aria-label={t('vault_trail_more')} onClick={toggle}>
        <MoreHorizontal className="a-ic xs" aria-hidden="true" />
      </button>
      {at && (
        <div className="a-menu vn-menu" role="menu" aria-label={t('vault_trail_more')} style={{ left: at.left, top: at.top }}>
          {hidden.map((part) => (
            <PartLink key={part.path} part={part} className="a-menu-item" onClick={() => setAt(null)} />
          ))}
        </div>
      )}
    </div>
  )
}

/** Up: the folder above, Vault home from a top-level folder, and dimmed on Vault home. */
function UpLink({ up }: { up: string | null }) {
  if (up === null) {
    return (
      <span className="a-ib vn-up is-off" aria-disabled="true" aria-label={t('vault_up', { name: VAULT_NAME })}>
        <ArrowUp className="a-ic sm" aria-hidden="true" />
      </span>
    )
  }
  const name = up === '' ? VAULT_NAME : noteName(up)
  const label = t('vault_up', { name })
  const icon = <ArrowUp className="a-ic sm" aria-hidden="true" />
  if (up === '') {
    return (
      <Link to="/notes" className="a-ib vn-up" aria-label={label} title={label}>
        {icon}
      </Link>
    )
  }
  return (
    <Link to="/notes/$" params={{ _splat: up }} className="a-ib vn-up" aria-label={label} title={label}>
      {icon}
    </Link>
  )
}

export interface LocationBarProps {
  /** The Vault path (a folder or a note); empty on Vault home. */
  path: string
  isFile: boolean
  /** The Trash and other pages that sit under Vault: the last part is plain text. */
  extra?: string
  treeOpen: boolean
  treeAvailable: boolean
  onToggleTree: () => void
}

/** Where you are in the Vault, with Up, and the button that opens the folder tree. */
export function LocationBar({ path, isFile, extra, treeOpen, treeAvailable, onToggleTree }: LocationBarProps) {
  const trail = trailOf(path, isFile)
  const parts = extra ? [...trail, { name: extra, path: '' }] : trail
  const up = extra ? '' : upTarget(path)
  const { before, hidden, after } = foldTrail(parts)
  const last = parts.length - 1
  const render = (part: TrailPart, index: number) => (
    <li key={`${index}:${part.path}`}>
      {index > 0 && <ChevronRight className="a-ic xs vn-sep" aria-hidden="true" />}
      {index === last ? (
        <span className="vn-crumb vn-current" aria-current="page">
          {part.name}
        </span>
      ) : (
        <PartLink part={part} className="vn-crumb" />
      )}
    </li>
  )
  return (
    <div className="vn-bar">
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
      {treeAvailable && (
        <button
          type="button"
          className="a-ib vn-toggle"
          aria-pressed={treeOpen}
          aria-controls="vault-tree"
          aria-label={treeOpen ? t('vault_tree_hide') : t('vault_tree_show')}
          title={`${treeOpen ? t('vault_tree_hide') : t('vault_tree_show')} (Ctrl B)`}
          onClick={onToggleTree}
        >
          <PanelLeft className="a-ic sm" aria-hidden="true" />
        </button>
      )}
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
  onToggle: (path: string, open: boolean) => void
  onShowAll: (path: string) => void
}

/** The folders of the Vault as a tree: one tab stop, arrows move, and the open path is bold. */
export function FolderTree({ root, loading, failed, current, open, showAll, onToggle, onShowAll }: FolderTreeProps) {
  const navigate = useNavigate()
  const rows = useMemo(() => treeRows(root, open, showAll), [root, open, showAll])
  const ancestors = useMemo(() => new Set(current ? ancestorPaths(`${current}/x`) : []), [current])
  const [focusPath, setFocusPath] = useState<string | null>(null)
  const tree = useRef<HTMLDivElement>(null)

  // Tab lands on the open row, or on the first when none is open.
  const tabStop = (() => {
    if (focusPath && rows.some((r) => r.path === focusPath)) return focusPath
    return rows.find((r) => r.path === current)?.path ?? rows[0]?.path ?? null
  })()

  // Keep the open row in view when it changes, such as after a link from a search result.
  const find = useCallback((selector: string) => tree.current?.querySelector(selector) as HTMLElement | null | undefined, [])
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
    if (row.kind === 'more') onShowAll(row.folder)
    else void navigate({ to: '/notes/$', params: { _splat: row.path } })
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
    <aside id="vault-tree" className="vn-tree" aria-label={t('notes_folders')}>
      <div className="vn-tree-head">
        <h2 className="a-label">{t('notes_folders')}</h2>
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
          {rows.map((row) => {
            const selected = row.path === current
            const className = ['vn-row', selected ? 'is-on' : '', ancestors.has(row.path) && !selected ? 'is-path' : ''].filter(Boolean).join(' ')
            const indent = { paddingInlineStart: `${8 + row.depth * 16}px` }
            if (row.kind === 'more') {
              return (
                <div key={`more:${row.path}`} role="treeitem" aria-level={row.depth + 1} aria-selected={false} data-path={row.path} tabIndex={row.path === tabStop ? 0 : -1} className="vn-row vn-more-row" style={indent} onFocus={() => setFocusPath(row.path)}>
                  <span className="vn-chev" />
                  <button type="button" tabIndex={-1} className="vn-name vn-show-all" onClick={() => onShowAll(row.folder)}>
                    {t('notes_see_all', { n: row.total })}
                  </button>
                </div>
              )
            }
            return (
              <div
                key={row.path}
                role="treeitem"
                aria-level={row.depth + 1}
                aria-selected={selected}
                aria-expanded={row.kind === 'folder' ? row.open : undefined}
                aria-label={row.kind === 'folder' ? t('vault_tree_count_label', { name: row.name, n: row.count }) : undefined}
                data-path={row.path}
                tabIndex={row.path === tabStop ? 0 : -1}
                className={className}
                style={indent}
                onFocus={() => setFocusPath(row.path)}
              >
                {row.kind === 'folder' ? (
                  <button type="button" tabIndex={-1} aria-hidden="true" className="vn-chev" onClick={() => onToggle(row.path, !row.open)}>
                    {row.open ? <ChevronDown className="a-ic xs" /> : <ChevronRight className="a-ic xs" />}
                  </button>
                ) : (
                  <span className="vn-chev" />
                )}
                <span className="vn-kind" aria-hidden="true">
                  {row.kind === 'folder' ? row.open ? <FolderOpen className="a-ic xs" /> : <Folder className="a-ic xs" /> : <FileText className="a-ic xs" />}
                </span>
                <Link to="/notes/$" params={{ _splat: row.path }} tabIndex={-1} className="vn-name">
                  {row.name}
                </Link>
                {row.kind === 'folder' && (
                  <span className="vn-count" aria-hidden="true">
                    {row.count}
                  </span>
                )}
              </div>
            )
          })}
        </div>
      )}
      <div className="vn-tree-foot">
        <Link to="/notes/trash" className="vn-row vn-trash">
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

/** Ctrl or Cmd B, unless the person is typing: the note editor uses it for bold. */
function isTreeShortcut(e: globalThis.KeyboardEvent): boolean {
  if (!(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey || e.key.toLowerCase() !== 'b') return false
  const el = e.target as HTMLElement | null
  return !(el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)))
}

/** Every Vault page sits in this: the location bar on top, the folder tree beside the page. */
export function VaultShell({ children }: { children?: ReactNode }) {
  const pathname = useRouterState({ select: (s) => s.location.pathname })
  const { path, special } = vaultLocation(pathname)
  const treeQuery = useQuery(noteTreeQuery(NOTEBOOKS))
  const root = treeQuery.data?.tree
  const atHome = path === '' && special === null
  const wide = useWindowAtLeast(WIDE_FROM)
  const [pref, setPref] = usePersistedState<boolean | null>('hq-vault-tree', null, (v) => v === null || typeof v === 'boolean')
  const treeOpen = pref ?? (wide && !atHome)
  const [openList, setOpenList] = usePersistedState<string[]>('hq-vault-tree-open', [], Array.isArray)
  const [showAll, setShowAll] = useState<PathSet>(new Set())
  const open = useMemo(() => new Set(openList), [openList])

  const node = findNode(root, path)
  const isFile = node ? node.type === 'file' : looksLikeFile(path)

  // Reveal: whatever way the person arrived, the folders above the open item are open.
  useEffect(() => {
    if (!path) return
    const need = ancestorPaths(isFile ? path : `${path}/x`).filter((p) => p !== NOTEBOOKS)
    setOpenList((prev) => (need.every((p) => prev.includes(p)) ? prev : [...prev, ...need.filter((p) => !prev.includes(p))].slice(-REMEMBER)))
  }, [path, isFile, setOpenList])

  const toggleTree = useCallback(() => setPref(!treeOpen), [setPref, treeOpen])
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (!isTreeShortcut(e)) return
      e.preventDefault()
      toggleTree()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [toggleTree])

  const onToggle = (p: string, wantOpen: boolean) =>
    setOpenList((prev) => (wantOpen ? (prev.includes(p) ? prev : [...prev, p].slice(-REMEMBER)) : prev.filter((x) => x !== p)))

  return (
    <div className="vn-shell">
      <LocationBar
        path={path}
        isFile={isFile}
        extra={special === 'trash' ? t('notes_trash') : undefined}
        treeOpen={treeOpen}
        treeAvailable
        onToggleTree={toggleTree}
      />
      <div className="vn-body">
        {treeOpen && (
          <FolderTree
            root={root}
            loading={treeQuery.isPending}
            failed={treeQuery.isError}
            current={special === null ? path : ''}
            open={open}
            showAll={showAll}
            onToggle={onToggle}
            onShowAll={(p) => setShowAll((prev) => new Set(prev).add(p))}
          />
        )}
        <div className="vn-main">{children ?? <Outlet />}</div>
      </div>
    </div>
  )
}

