import { t } from '~/strings'
import { createFileRoute, useParams } from '@tanstack/react-router'
import { Archive, Loader2, PanelLeftClose, PanelLeftOpen, Plus, RefreshCw, Terminal } from 'lucide-react'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useMemo, useState } from 'react'
import { NewAgentDialog } from '~/components/sessions/NewAgentDialog'
import { LockdownBanner, StopAllButton } from '~/components/sessions/EmergencyStop'
import { SessionDetail } from '~/components/sessions/SessionDetail'
import { SessionInspector } from '~/components/sessions/SessionInspector'
import { SessionRow } from '~/components/sessions/SessionRow'
import { usePolled } from '~/components/sessions/usePolled'
import { globalSessionsApi, workbenchApi, type HarnessSession } from '~/lib/sessionsApi'
import { useHQStore } from '~/store/hqStore'
import { needsYouCount, sessionTitle, statusInfo } from '~/lib/workbench'
import { GROUP_WORDS, HELPER_CHIPS, chipCounts, chipWords, finishedToArchive, groupHelpers, taskBanner, type HelperChip } from '~/lib/helperView'
import { edgeWorkerApi, ownCardId, ownTitle, ownWords, type EdgeRun } from '~/lib/edgeWorker'
import { toastUndo } from '~/lib/toast'
import { useThreadStore } from '~/store/threadStore'
import { tn } from '~/strings'
import { useSidebarLayout } from '~/components/sessions/useSidebarLayout'
import { SIDEBAR_MAX, SIDEBAR_MIN } from '~/lib/sidebarLayout'
import { ErrorText } from '~/components/sessions/ErrorText'

const HELPER_EVENTS = ['helpers:change', 'sessions:sync'] as const
const LIST_POLL_MS = 8_000
const DETAIL_POLL_MS = 5_000
/** Each computer is asked over the network, so its lockdown state is read less often. */
const HOSTS_POLL_MS = 60_000
const OWN_POLL_MS = 20_000

interface SessionsSearch {
  id?: string
  task?: string
}

export const Route = createFileRoute('/helpers')({
  validateSearch: (search: Record<string, unknown>): SessionsSearch => {
    const text = (v: unknown) => (typeof v === 'string' && v ? v : undefined)
    const id = text(search.id)
    const task = text(search.task)
    return { ...(id ? { id } : {}), ...(task ? { task } : {}) }
  },
  component: WorkbenchPage,
})

const NEW_AGENT_BUTTON_CLASS = 'hq-btn-primary'

function WorkbenchPage() {
  const { task } = Route.useSearch()
  const selectedId = useParams({ strict: false }).id
  const navigate = Route.useNavigate()
  const [showArchived, setShowArchived] = useState(false)
  const [creating, setCreating] = useState(false)
  const sidebar = useSidebarLayout()
  // Archived agents are always fetched and hidden here, so the toggle is instant and never blanks the list.
  const filters = useMemo(() => ({ ...(task ? { task_id: task } : {}), include_archived: true }), [task])
  const list = usePolled(JSON.stringify(filters), () => globalSessionsApi.list(filters), LIST_POLL_MS, true, true, HELPER_EVENTS)
  const detail = usePolled(selectedId ?? '', () => globalSessionsApi.get(selectedId ?? ''), DETAIL_POLL_MS, Boolean(selectedId), true, HELPER_EVENTS)
  const hosts = usePolled('hosts', () => workbenchApi.hosts(), HOSTS_POLL_MS, true, true, HELPER_EVENTS)
  const afterStop = () => void Promise.all([list.refresh(), hosts.refresh()])

  // An unfiltered list is the nav badge's source, so the nav stops polling while this page is open.
  const setCount = useHQStore((st) => st.setNeedsYouCount)
  const setFeeds = useHQStore((st) => st.setWorkbenchFeedsCount)
  const feeds = !task
  useEffect(() => {
    setFeeds(feeds)
    return () => setFeeds(false)
  }, [feeds, setFeeds])
  useEffect(() => {
    if (!feeds) return
    if (list.error !== null) setCount(0)
    else if (list.data) setCount(needsYouCount(list.data))
  }, [feeds, list.data, list.error, setCount])

  const select = (id: string | undefined) =>
    void navigate({ to: id ? '/helpers/$id' : '/helpers', params: id ? { id } : {}, search: (prev) => ({ ...prev, id: undefined }), replace: true })
  const refreshAll = async () => {
    await Promise.all([list.refresh(), detail.refresh()])
  }
  // The detail fetch is fresher than the list, and also covers an agent the list's filters hide.
  const selected: HarnessSession | null = detail.data ?? list.data?.find((s) => s.id === selectedId) ?? null

  const collapsed = sidebar.collapsed
  return (
    <div className="flex h-full min-h-0 w-full max-w-full overflow-hidden">
      <aside
        aria-label={t('helpers_list_aria')}
        style={{ '--wb-w': collapsed ? '72px' : `${sidebar.width}px` } as React.CSSProperties}
        className={`${selectedId ? 'hidden min-[1024px]:flex' : 'flex'} flex-col w-full min-[1024px]:w-[var(--wb-w)] min-[1024px]:border-r md:border-white/10 min-h-0 shrink-0 ${sidebar.dragging ? '' : 'md:transition-[width] md:duration-200'}`}
      >
        {collapsed ? (
          <CollapsedRail list={list.data ?? []} selectedId={selectedId} onSelect={select} onNew={() => setCreating(true)} onExpand={sidebar.toggle} onStopped={afterStop} />
        ) : (
          <>
            <div className="flex items-center gap-2 px-4 pt-4 pb-3">
              <h1 className="text-xl font-semibold tracking-tight text-neutral-50 mr-auto" style={{ fontFamily: 'var(--font-heading)' }}>{t('place_helpers')}</h1>
              <button
                type="button"
                onClick={() => void list.refresh()}
                className="hq-icon-btn"
                aria-label={t('helpers_refresh')}
              >
                <RefreshCw className="w-4 h-4" />
              </button>
              <span className="hidden md:contents">
                <button type="button" onClick={sidebar.toggle} className="hq-icon-btn" aria-label={t('helpers_collapse')}>
                  <PanelLeftClose className="w-4 h-4" />
                </button>
              </span>
            </div>
            <div className="flex gap-2 px-4 pb-3">
              <button type="button" onClick={() => setCreating(true)} className="hq-btn-primary flex-1 min-w-0">
                <Plus className="w-4 h-4" />
                {t('helpers_start_a_helper')}
              </button>
              <StopAllButton onStopped={afterStop} />
            </div>
            <LockdownBanner hosts={hosts.data?.hosts ?? []} />
            <AgentList list={list} selectedId={selectedId} showArchived={showArchived} onShowArchived={setShowArchived} onNew={() => setCreating(true)} task={task} onShowAll={() => void navigate({ search: (prev) => ({ ...prev, task: undefined }), replace: true })} />
          </>
        )}
      </aside>
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={t('helpers_resize')}
        aria-valuemin={SIDEBAR_MIN}
        aria-valuemax={SIDEBAR_MAX}
        aria-valuenow={collapsed ? 72 : sidebar.width}
        tabIndex={0}
        onPointerDown={collapsed ? undefined : sidebar.startDrag}
        onDoubleClick={sidebar.reset}
        onKeyDown={sidebar.onKeyDown}
        className={`hq-resize-handle hidden md:block ${sidebar.dragging ? 'is-dragging' : ''} ${collapsed ? 'is-disabled' : ''}`}
      />
      <main className={`${selectedId ? 'flex' : 'hidden min-[1024px]:flex'} flex-1 min-w-0 min-h-0 flex-col`}>
        <div className="flex flex-col flex-1 min-h-0 min-w-0 overflow-hidden">
          <div className="mx-auto flex flex-1 min-h-0 min-w-0 w-full max-w-[1500px]">
            <div className="flex flex-col flex-1 min-h-0 min-w-0 max-w-[760px] mx-auto">
              {selectedId ? (
                selected ? (
                  <SessionDetail key={selected.id} session={selected} onBack={() => select(undefined)} onChanged={refreshAll} />
                ) : (
                  <DetailPlaceholder error={detail.error} loading={detail.loading} onBack={() => select(undefined)} />
                )
              ) : (
                <div className="m-auto flex flex-col items-center gap-3 text-neutral-400 text-sm p-6 text-center">
                  <span className="flex items-center justify-center w-12 h-12 rounded-2xl hq-glass-card">
                    <Terminal className="w-5 h-5" />
                  </span>
                  {t('helpers_empty_pick')}
                </div>
              )}
            </div>
            {selected && (
              <div className="hidden min-[1400px]:flex">
                <SessionInspector session={selected} onChanged={refreshAll} />
              </div>
            )}
          </div>
        </div>
      </main>
      {creating && (
        <NewAgentDialog
          onClose={() => setCreating(false)}
          onStarted={(id) => {
            setCreating(false)
            select(id)
            void list.refresh()
          }}
        />
      )}
    </div>
  )
}

interface ListProps {
  list: ReturnType<typeof usePolled<HarnessSession[]>>
  selectedId: string | undefined
  showArchived: boolean
  onShowArchived: (show: boolean) => void
  onNew: () => void
  task: string | undefined
  onShowAll: () => void
}

function SectionTitle({ children, count }: { children: string; count: number }) {
  return (
    <h2 className="flex items-center gap-2 px-2 pt-4 pb-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400">
      {children}
      <span className="px-1.5 rounded-full bg-white/10 text-[10px] text-neutral-300">{count}</span>
    </h2>
  )
}

function AgentList({ list, selectedId, showArchived, onShowArchived, onNew, task, onShowAll }: ListProps) {
  const [archiveError, setArchiveError] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [chip, setChip] = useState<HelperChip>('all')
  const own = usePolled('edge-worker', () => edgeWorkerApi.runs(), OWN_POLL_MS, true, true, HELPER_EVENTS)
  const sessions = list.data ?? []
  const groups = useMemo(() => groupHelpers(sessions, chip, showArchived), [sessions, chip, showArchived])
  const counts = useMemo(() => chipCounts(sessions), [sessions])
  const banner = task ? taskBanner(sessions, task) : null
  const toArchive = useMemo(() => finishedToArchive(sessions).map((s) => s.id), [sessions])

  const archive = async (ids: string[], archived: boolean) => {
    setBusyId(ids.length === 1 ? ids[0] : 'all')
    setArchiveError(null)
    try {
      await Promise.all(ids.map((id) => globalSessionsApi.archive(id, archived)))
      return true
    } catch {
      setArchiveError(t('helpers_archive_failed'))
      return false
    } finally {
      setBusyId(null)
      await list.refresh()
    }
  }
  // Archiving acts at once; the toast carries the way back.
  const archiveFinished = async () => {
    const ids = toArchive
    if (await archive(ids, true)) {
      toastUndo(tn('helpers_archived_n', ids.length), () => Promise.all(ids.map((id) => globalSessionsApi.archive(id, false))).then(() => list.refresh()), t('helpers_archived_back', { n: ids.length }))
    }
  }

  if (list.loading) {
    return (
      <div className="flex items-center gap-2 p-4 text-xs text-neutral-500" role="status">
        <Loader2 className="w-4 h-4 animate-spin" aria-hidden="true" />
        {t('helpers_loading')}
      </div>
    )
  }
  if (!list.data) {
    return <ErrorText className="p-4 text-xs">{list.error ?? t('helpers_load_failed')}</ErrorText>
  }

  const ownRuns = own.data ?? []
  const empty = sessions.filter((s) => !s.archived).length === 0 && ownRuns.length === 0

  return (
    <div className="flex-1 overflow-y-auto overscroll-contain min-h-0 pb-4 px-2">
      {banner && (
        <div className="px-2 py-2 text-xs text-neutral-300 border-y border-white/5" role="status">
          <p className="font-semibold">{banner.title}</p>
          <p>{banner.line}</p>
          <button type="button" onClick={onShowAll} className="a-btn secondary mt-1">
            {t('helpers_show_all')}
          </button>
        </div>
      )}
      <div role="group" aria-label={t('helpers_filters_aria')} className="flex flex-wrap gap-1.5 px-1 py-2">
        {HELPER_CHIPS.map((c) => (
          <button key={c} type="button" aria-pressed={chip === c} onClick={() => setChip(c)} className={`a-chip ${chip === c ? 'is-on' : ''} min-h-11 px-3 rounded-full border border-white/10 text-xs`}>
            {t(chipWords(c))} · {counts[c]}
          </button>
        ))}
      </div>
      {list.error && <ErrorText className="px-3 py-1.5">{t('helpers_refresh_failed', { reason: list.error })}</ErrorText>}
      {archiveError && <ErrorText className="px-3 py-1.5">{archiveError}</ErrorText>}
      {empty && (
        <div className="flex flex-col items-center gap-3 p-6 text-center">
          <Terminal className="w-6 h-6 text-neutral-500" aria-hidden="true" />
          <p className="text-sm text-neutral-200">{t('helpers_empty_title')}</p>
          <p className="text-xs text-neutral-400">{t('helpers_empty_body')}</p>
          <button type="button" onClick={onNew} className={NEW_AGENT_BUTTON_CLASS}>
            <Plus className="w-3.5 h-3.5" aria-hidden="true" />
            {t('helpers_start_a_helper')}
          </button>
        </div>
      )}
      {!empty && groups.length === 0 && chip !== 'all' && <p className="px-3 py-4 text-xs text-neutral-400">{t('helpers_empty_chip')}</p>}
      {groups.map(({ group, items }) => (
        <section key={group} aria-label={t(GROUP_WORDS[group])} data-group={group}>
          <SectionTitle count={items.length}>{t(GROUP_WORDS[group])}</SectionTitle>
          {group === 'past' && toArchive.length > 0 && (
            <div className="px-2 pb-2">
              <button type="button" disabled={busyId !== null} onClick={() => void archiveFinished()} className="a-btn secondary">
                <Archive className="w-3.5 h-3.5" aria-hidden="true" />
                {t('helpers_archive_finished')}
              </button>
            </div>
          )}
          <ul>
            {items.map((s) => (
              <SessionRow
                key={s.id}
                session={s}
                selected={s.id === selectedId}
                onAnswered={() => list.refresh()}
                onRestore={s.archived ? () => void archive([s.id], false) : undefined}
                busy={busyId === s.id || busyId === 'all'}
              />
            ))}
          </ul>
        </section>
      ))}
      {chip === 'all' && ownRuns.length > 0 && <OwnWork runs={ownRuns} />}
      <label className="flex items-center gap-2 px-3 min-h-11 text-xs text-neutral-400 cursor-pointer">
        <input type="checkbox" checked={showArchived} onChange={(e) => onShowArchived(e.target.checked)} className="h-4 w-4" style={{ accentColor: 'var(--accent-green)' }} />
        {t('helpers_show_archived')}
      </label>
    </div>
  )
}

/** What HQ itself is doing for tasks. A card opens its chat; there is no Stop, no Rename and no screen. */
function OwnWork({ runs }: { runs: EdgeRun[] }) {
  const navigate = useNavigate()
  return (
    <section aria-label={t('helpers_own_work')} data-group="own">
      <SectionTitle count={runs.length}>{t('helpers_own_work')}</SectionTitle>
      <ul>
        {runs.map((run) => (
          <li key={ownCardId(run)} className="hq-row mb-1.5">
            <button
              type="button"
              disabled={!run.thread_id}
              onClick={() => {
                if (!run.thread_id) return
                useThreadStore.getState().setActiveThread(run.thread_id)
                void navigate({ to: '/chat' })
              }}
              aria-label={`${ownTitle(run)}, ${ownWords(run)}. ${t('helpers_own_open_chat')}`}
              className="w-full text-left px-3.5 py-3 min-h-11 space-y-1 rounded-xl"
            >
              <span className="block text-sm font-semibold text-neutral-100 truncate">{ownTitle(run)}</span>
              <span className="block text-xs text-neutral-400">{ownWords(run)}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  )
}

function DetailPlaceholder({ error, loading, onBack }: { error: string | null; loading: boolean; onBack: () => void }) {
  if (loading) {
    return (
      <div className="m-auto flex items-center gap-2 text-xs font-mono text-neutral-500" role="status">
        <Loader2 className="w-4 h-4 animate-spin" />
        {t('helpers_open_loading')}
      </div>
    )
  }
  return (
    <div className="m-auto flex flex-col items-center gap-2 p-6 text-center text-xs font-mono">
      <ErrorText className="text-xs">{error ?? t('helpers_open_missing')}</ErrorText>
      <button type="button" onClick={onBack} className="h-9 px-3 rounded border border-white/10 text-neutral-300 hover:bg-white/10">
        {t('helpers_back')}
      </button>
    </div>
  )
}

interface RailProps {
  list: HarnessSession[]
  selectedId: string | undefined
  onSelect: (id: string) => void
  onNew: () => void
  onExpand: () => void
  onStopped: () => void
}

const initials = (title: string) =>
  title
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? '')
    .join('')

/** The sidebar folded down to one avatar per agent, with a status dot, so nothing that needs you is hidden. */
function CollapsedRail({ list, selectedId, onSelect, onNew, onExpand, onStopped }: RailProps) {
  const visible = list.filter((s) => !s.archived).slice(0, 14)
  return (
    <div className="flex flex-col items-center gap-2.5 py-4 overflow-y-auto min-h-0">
      <button type="button" onClick={onExpand} className="hq-icon-btn" aria-label={t('helpers_expand')}>
        <PanelLeftOpen className="w-4 h-4" />
      </button>
      <button type="button" onClick={onNew} className="hq-btn-primary !h-11 !w-11 !p-0" aria-label={t('helpers_new_aria')}>
        <Plus className="w-4 h-4" />
      </button>
      <StopAllButton compact onStopped={onStopped} />
      {visible.map((s) => {
        const status = statusInfo(s)
        const title = sessionTitle(s)
        const tone = s.agent_status === 'blocked' || status.warn ? 'var(--accent-blue)' : s.status === 'running' ? 'var(--accent-green)' : 'var(--text-dim)'
        return (
          <button
            key={s.id}
            type="button"
            onClick={() => onSelect(s.id)}
            aria-label={`${title}, ${status.word}`}
            aria-current={s.id === selectedId ? 'true' : undefined}
            title={`${title} · ${status.word}`}
            className={`hq-avatar ${s.id === selectedId ? 'is-selected' : ''}`}
          >
            {initials(title)}
            <span className="hq-avatar-dot" style={{ background: tone }} />
          </button>
        )
      })}
    </div>
  )
}
