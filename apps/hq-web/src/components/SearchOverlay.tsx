import './search.css'
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { Search, X } from 'lucide-react'
import { t } from '~/strings'
import { useHQStore } from '~/store/hqStore'
import { useThreadStore } from '~/store/threadStore'
import { useSharedConnection } from '~/lib/connection'
import { useEdition } from '~/lib/edition'
import { globalSessionsApi } from '~/lib/sessionsApi'
import { STATUS_LABELS } from '~/lib/tasksApi'
import { agentName, computerName } from '~/lib/names'
import { sessionTitle, statusInfo } from '~/lib/workbench'
import { relTime } from '~/lib/time'
import { toastUndo } from '~/lib/toast'
import { currentHqId } from '~/lib/savedCopy'
import { NEW_CHAT_KEY, restoreDraft } from '~/components/chat/draftStore'
import {
  ASK_MAX,
  LITE_SCOPES,
  SCOPES,
  foundParts,
  folderLabel,
  folderTrail,
  matchChats,
  matchHelpers,
  matchTasks,
  readRecent,
  searchFailureLine,
  topFolders,
  visibleRows,
  withRecent,
  writeRecent,
  type SearchScope,
} from '~/lib/search'
import type { SearchHit } from '~/lib/vaultApi'
import { useVaultSearch } from './useVaultSearch'
import { forgetSearch, readBack, rememberSearch } from '~/lib/searchBack'

type Kind = Exclude<SearchScope, 'all'>

interface Row {
  id: string
  title: string
  sub: string
  trail?: string
  pill?: string
  open: () => void
}

interface OpenDetail {
  scope?: SearchScope
  folder?: string
  q?: string
  scroll?: number
}

const CHIP_KEYS = {
  all: 'search_chip_all',
  notes: 'search_chip_notes',
  tasks: 'search_chip_tasks',
  chats: 'search_chip_chats',
  helpers: 'search_chip_helpers',
} as const

const FIELD_KEYS = {
  all: 'search_field_all',
  notes: 'search_field_notes',
  tasks: 'search_field_tasks',
  chats: 'search_field_chats',
  helpers: 'search_field_helpers',
} as const

const NONE_KEYS = {
  notes: 'search_none_notes',
  tasks: 'search_none_tasks',
  chats: 'search_none_chats',
  helpers: 'search_none_helpers',
} as const

const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })

function Found({ text, query }: { text: string; query: string }) {
  return (
    <>
      {foundParts(text, query).map((p, i) => (p.found ? <mark key={i}>{p.text}</mark> : <Fragment key={i}>{p.text}</Fragment>))}
    </>
  )
}

/** The one Search: Ctrl or Cmd K, the top bar field and the Notes pill open it over whatever place you are in. */
export function SearchOverlay() {
  const [open, setOpen] = useState(false)
  const [start, setStart] = useState<OpenDetail>({})
  const opener = useRef<Element | null>(null)

  useEffect(() => {
    const show = (detail: OpenDetail) => {
      opener.current = document.activeElement
      setStart(detail)
      setOpen(true)
    }
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        if (open) setOpen(false)
        else show({})
      }
    }
    const onOpen = (e: Event) => show((e as CustomEvent<OpenDetail | null>).detail ?? {})
    // Back from an opened result lands on the entry marked at that time: Search reopens once, as it was left.
    const onPop = () => {
      const back = readBack(window.location.search, window.history.state)
      if (!back) return
      forgetSearch()
      show({ q: back.q, scope: back.scope, ...(back.folder ? { folder: back.folder } : {}), scroll: back.scroll })
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('hq:open-search', onOpen)
    window.addEventListener('popstate', onPop)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('hq:open-search', onOpen)
      window.removeEventListener('popstate', onPop)
    }
  }, [open])

  const close = useCallback(() => {
    setOpen(false)
    const back = opener.current
    if (back instanceof HTMLElement) requestAnimationFrame(() => back.focus())
  }, [])

  if (!open) return null
  return <SearchPanel start={start} close={close} />
}

function SearchPanel({ start, close }: { start: OpenDetail; close: () => void }) {
  const lite = useEdition() === 'lite'
  const scopes = lite ? LITE_SCOPES : SCOPES
  const navigate = useNavigate()
  const connection = useSharedConnection()
  const inputRef = useRef<HTMLInputElement>(null)
  const [scope, setScope] = useState<SearchScope>(start.folder ? 'notes' : (start.scope ?? 'all'))
  const [folder, setFolder] = useState<string | null>(start.folder ?? null)
  const [expanded, setExpanded] = useState<Kind | null>(null)
  const [selected, setSelected] = useState(0)
  const hqId = currentHqId()
  const [recent, setRecent] = useState(() => readRecent(hqId))

  const notes = useVaultSearch(() => undefined, folder, start.q)
  const { query, setQuery, again } = notes
  const typed = query.trim().length > 0

  const tasks = useHQStore((s) => s.tasks)
  const threads = useThreadStore((s) => s.threads)
  const helpersQuery = useQuery({
    queryKey: ['search', 'helpers'],
    queryFn: () => globalSessionsApi.list({}),
    enabled: !lite,
    staleTime: 30_000,
  })

  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  // HQ coming back runs a search that failed again, without a tap.
  const wasFailed = useRef(false)
  useEffect(() => {
    if (connection === 'connected' && wasFailed.current) again()
    wasFailed.current = notes.failed
  }, [connection, notes.failed, again])

  const wantsNotes = scope === 'all' || scope === 'notes'
  const wantsChats = !lite && (scope === 'all' || scope === 'chats')
  const wantsTasks = scope === 'all' || scope === 'tasks'
  const wantsHelpers = !lite && (scope === 'all' || scope === 'helpers')
  const notesFailed = typed && wantsNotes && notes.failed

  const taskHits = useMemo(() => (typed && wantsTasks ? matchTasks(tasks, query) : []), [typed, wantsTasks, tasks, query])
  const helperHits = useMemo(
    () => (typed && wantsHelpers ? matchHelpers(helpersQuery.data ?? [], query) : []),
    [typed, wantsHelpers, helpersQuery.data, query],
  )
  const chatHits = useMemo(() => (typed && wantsChats ? matchChats(threads, query) : []), [typed, wantsChats, threads, query])
  const noteHits: SearchHit[] = typed && wantsNotes && !notes.failed ? notes.results : []

  const listRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (start.scroll && listRef.current) listRef.current.scrollTop = start.scroll
  }, [start.scroll, noteHits.length, taskHits.length])

  const go = useCallback(
    (row: Row) => {
      setRecent((r) => writeRecent(hqId, withRecent(r, query)))
      if (query.trim()) rememberSearch({ q: query, scope, folder, scroll: listRef.current?.scrollTop ?? 0 })
      close()
      row.open()
    },
    [hqId, query, scope, folder, close],
  )

  const rows = useMemo<Record<Kind, Row[]>>(
    () => ({
      notes: noteHits.map((h) => ({
        id: `note:${h.notePath}`,
        title: h.title,
        trail: folderTrail(h.notePath.split('/').slice(0, -1).join('/')),
        sub: h.snippet,
        open: () => void navigate({ to: '/notes/$', params: { _splat: h.notePath } }),
      })),
      tasks: taskHits.map((x) => ({
        id: `task:${x.id}`,
        title: x.title,
        sub: t('search_task_line', { id: x.display_id, status: STATUS_LABELS[x.status] }),
        open: () => void navigate({ to: '/tasks/$id', params: { id: x.id } }),
      })),
      chats: chatHits.map((x) => ({
        id: `chat:${x.threadId}`,
        title: x.title,
        sub: [relTime(x.updatedAt), x.lastMessagePreview].filter(Boolean).join(' · '),
        open: () => void navigate({ to: '/chat/$id', params: { id: x.threadId } }),
      })),
      helpers: helperHits.map((s) => {
        const info = statusInfo(s)
        return {
          id: `helper:${s.id}`,
          title: sessionTitle(s),
          sub: t('search_helper_line', { agent: agentName(s.harness), computer: computerName(s.host), state: info.word }),
          pill: info.warn || info.agent === 'blocked' ? info.word : undefined,
          open: () => void navigate({ to: '/helpers/$id', params: { id: s.id } }),
        }
      }),
    }),
    [noteHits, taskHits, chatHits, helperHits, navigate],
  )

  const order: Kind[] = scope === 'all' ? ['notes', 'tasks', 'chats', 'helpers'] : [scope]
  const shown = order.map((kind) => ({
    kind,
    all: rows[kind],
    rows: visibleRows(rows[kind], scope !== 'all' || expanded === kind),
  }))
  const flat = shown.flatMap((g) => g.rows)
  const active = Math.min(selected, Math.max(flat.length - 1, 0))
  const total = rows.notes.length + rows.tasks.length + rows.chats.length + rows.helpers.length

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      close()
    } else if (e.key === 'ArrowDown') {
      e.preventDefault()
      setSelected((i) => (flat.length ? (i + 1) % flat.length : 0))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setSelected((i) => (flat.length ? (i - 1 + flat.length) % flat.length : 0))
    } else if (e.key === 'Enter' && flat[active]) {
      e.preventDefault()
      go(flat[active])
    }
  }

  const pickScope = (next: SearchScope) => {
    setScope(next)
    setExpanded(null)
    setSelected(0)
  }

  const clearRecent = () => {
    const before = recent
    setRecent(writeRecent(hqId, []))
    toastUndo(t('search_recent_cleared'), async () => setRecent(writeRecent(hqId, before)), t('search_recent'))
  }

  const askNotes = noteHits.slice(0, ASK_MAX)
  // Chat has no attach seam yet: the notes go into the new chat's draft as note references.
  const ask = () => {
    restoreDraft(NEW_CHAT_KEY, askNotes.map((h) => `\`${h.notePath}\``).join('\n'))
    close()
    void navigate({ to: '/chat', search: { compose: '1' } })
  }

  const folders = scope === 'notes' && !folder ? topFolders(notes.results) : []
  const placeholder = lite
    ? t('search_field_lite')
    : folder
      ? t('search_field_notes_in', { folder: folderLabel(folder) })
      : t(FIELD_KEYS[scope])
  const savedAt = helpersQuery.dataUpdatedAt ? clock(helpersQuery.dataUpdatedAt) : null
  const countOf = (s: SearchScope) => (s === 'all' ? total : rows[s].length)
  const countKnown = (s: SearchScope) => typed && !(notesFailed && (s === 'notes' || s === 'chats'))
  const listId = 'hq-search-list'

  return (
    <>
      <div className="a-scrim sr-scrim" onClick={close} />
      <div role="dialog" aria-modal="true" aria-label={t('search_open_label')} className="sr-panel">
        <div className="sr-top">
          <label className="a-search">
          <Search className="a-ic sm" aria-hidden="true" />
          <input
            ref={inputRef}
            type="text"
            role="combobox"
            aria-expanded={flat.length > 0}
            aria-controls={listId}
            aria-activedescendant={flat.length > 0 ? `hq-search-row-${active}` : undefined}
            aria-label={placeholder}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              setSelected(0)
            }}
            onKeyDown={onKeyDown}
            placeholder={placeholder}
            enterKeyHint="go"
          />
          </label>
          <button type="button" onClick={close} aria-label={t('search_close')} className="a-ib">
            <X className="a-ic" aria-hidden="true" />
          </button>
        </div>

        <div role="tablist" aria-label={t('search_chips_label')} className="a-chips sr-chips">
          {scopes.map((s) => (
            <button key={s} type="button" role="tab" aria-selected={scope === s} className={`a-chip${scope === s ? ' is-on' : ''}`} onClick={() => pickScope(s)}>
              {t(CHIP_KEYS[s])}
              {countKnown(s) && <em>{countOf(s)}</em>}
            </button>
          ))}
          {folder && (
            <button type="button" className="a-chip is-on" onClick={() => setFolder(null)} aria-label={t('search_chip_in_folder_clear', { folder: folderLabel(folder) })}>
              {t('search_chip_in_folder', { folder: folderLabel(folder) })} <X className="a-ic xs" aria-hidden="true" />
            </button>
          )}
          {folders.length > 0 && (
            <>
              <button type="button" className="a-chip is-on" aria-pressed="true">
                {t('search_chip_all_folders')}
              </button>
              {folders.map((f) => (
                <button key={f.folder} type="button" className="a-chip" onClick={() => setFolder(f.folder)}>
                  {f.folder} <em>{f.count}</em>
                </button>
              ))}
            </>
          )}
        </div>

        {folder && (
          <p className="sr-note">
            {t('search_folder_note', { folder: folderTrail(folder) })}
          </p>
        )}

        <div ref={listRef} id={listId} role="listbox" aria-label={t('search_results_label')} className="sr-body">
          {!typed && (
            <div>
              {recent.length > 0 && (
                <div>
                  <div className="sr-recent">
                    <p className="a-body-b">{t('search_recent')}</p>
                    <button type="button" className="a-btn quiet compact" onClick={clearRecent}>
                      {t('search_recent_clear')}
                    </button>
                  </div>
                  <ul>
                    {recent.map((r) => (
                      <li key={r} className="a-row">
                        <button type="button" className="a-row-main" onClick={() => setQuery(r)}>
                          <b>{r}</b>
                        </button>
                        <button type="button" className="a-ib sm" aria-label={t('search_recent_remove', { query: r })} onClick={() => setRecent(writeRecent(hqId, recent.filter((x) => x !== r)))}>
                          <X className="a-ic xs" aria-hidden="true" />
                        </button>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              <p className="a-body-sm a-muted sr-pad">{scope === 'notes' ? t('search_hint_notes') : t('search_hint_empty')}</p>
              {lite && <p className="a-cap a-muted sr-pad">{t('search_lite_note')}</p>}
            </div>
          )}

          {notesFailed && (
            <div className="sr-down" role="status">
              <p className="a-warn-t">{searchFailureLine(connection, lite)}</p>
              {!lite && savedAt && <p className="a-muted">{t('search_saved_note', { time: savedAt })}</p>}
              {!lite && <p className="a-muted">{t('search_back_note')}</p>}
            </div>
          )}

          {typed && wantsNotes && notes.isLoading && !notes.failed && (
            <p className="a-loading sr-pad" role="status">
              <span className="a-spin" aria-hidden="true" />
              {t('search_loading')}
            </p>
          )}

          {typed &&
            shown.map((g, gi) => {
              if (g.all.length === 0) return null
              const offset = shown.slice(0, gi).reduce((n, x) => n + x.rows.length, 0)
              const heading = t(CHIP_KEYS[g.kind])
              return (
                <section key={g.kind} aria-label={t('search_group_label', { kind: heading, n: g.all.length })}>
                  <h3 className="sr-grp">
                    {heading} <span className="a-count">{g.all.length}</span>
                  </h3>
                  {g.rows.map((r, i) => {
                    const idx = offset + i
                    const on = idx === active
                    return (
                      <div
                        key={r.id}
                        id={`hq-search-row-${idx}`}
                        role="option"
                        aria-selected={on}
                        onClick={() => go(r)}
                        onMouseMove={() => setSelected(idx)}
                        className="a-row sr-hit"
                      >
                        <span className="a-row-main">
                          <b>
                            <Found text={r.title} query={query} />
                          </b>
                          {r.trail && <span className="sr-trail">{r.trail}</span>}
                          {r.sub && (
                            <span>
                              <Found text={r.sub} query={query} />
                            </span>
                          )}
                        </span>
                        {r.pill && <span className="a-state needs">{r.pill}</span>}
                      </div>
                    )
                  })}
                  {g.all.length > g.rows.length && (
                    <button
                      type="button"
                      className="a-btn quiet compact sr-pad"
                      onClick={() => {
                        pickScope(g.kind)
                        setExpanded(g.kind)
                      }}
                    >
                      {t('search_show_all', { n: g.all.length })}
                    </button>
                  )}
                  {g.kind === 'notes' && !lite && askNotes.length > 0 && (
                    <div className="sr-ask">
                    <button type="button" className="a-btn secondary compact" onClick={ask}>
                      {noteHits.length === 1
                        ? t('search_ask_one')
                        : noteHits.length > ASK_MAX
                          ? t('search_ask_first', { n: ASK_MAX })
                          : t('search_ask_some', { n: noteHits.length })}
                    </button>
                    </div>
                  )}
                </section>
              )
            })}

          {typed && !notes.isLoading && total === 0 && !notesFailed && (
            <div className="a-empty">
              {scope === 'all' ? (
                <>
                  <b>{t('search_none_any')}</b>
                  <p>{t('search_none_try')}</p>
                </>
              ) : (
                <>
                  <b>{t(NONE_KEYS[scope])}</b>
                  <button type="button" className="a-btn secondary" onClick={() => pickScope('all')}>
                    {t('search_everything')}
                  </button>
                </>
              )}
            </div>
          )}
        </div>

        <div className="sr-foot">
          <span>{t('search_key_up_down')}</span>
          <span>{t('search_key_enter')}</span>
          <span>{t('search_key_tab')}</span>
          <span>{t('search_key_esc')}</span>
        </div>
      </div>
    </>
  )
}
