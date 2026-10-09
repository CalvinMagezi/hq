import { createFileRoute } from '@tanstack/react-router'
import { Archive, Loader2, PanelLeftClose, PanelLeftOpen, Plus, RefreshCw, Terminal } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { NewAgentDialog } from '~/components/sessions/NewAgentDialog'
import { SessionDetail } from '~/components/sessions/SessionDetail'
import { SessionInspector } from '~/components/sessions/SessionInspector'
import { SessionRow } from '~/components/sessions/SessionRow'
import { usePolled } from '~/components/sessions/usePolled'
import { globalSessionsApi, type HarnessSession } from '~/lib/sessionsApi'
import { useHQStore } from '~/store/hqStore'
import { archivable, groupSessions, needsYouCount, sessionTitle, statusInfo } from '~/lib/workbench'
import { useSidebarLayout } from '~/components/sessions/useSidebarLayout'
import { SIDEBAR_MAX, SIDEBAR_MIN } from '~/lib/sidebarLayout'
import { ErrorText } from '~/components/sessions/ErrorText'

const LIST_POLL_MS = 8_000
const DETAIL_POLL_MS = 5_000

interface SessionsSearch {
  id?: string
  task?: string
}

export const Route = createFileRoute('/sessions')({
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
  const { id: selectedId, task } = Route.useSearch()
  const navigate = Route.useNavigate()
  const [showArchived, setShowArchived] = useState(false)
  const [creating, setCreating] = useState(false)
  const sidebar = useSidebarLayout()
  // Archived agents are always fetched and hidden here, so the toggle is instant and never blanks the list.
  const filters = useMemo(() => ({ ...(task ? { task_id: task } : {}), include_archived: true }), [task])
  const list = usePolled(JSON.stringify(filters), () => globalSessionsApi.list(filters), LIST_POLL_MS, true, true)
  const detail = usePolled(selectedId ?? '', () => globalSessionsApi.get(selectedId ?? ''), DETAIL_POLL_MS, Boolean(selectedId), true)

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

  const select = (id: string | undefined) => void navigate({ search: (prev) => ({ ...prev, id }), replace: true })
  const refreshAll = async () => {
    await Promise.all([list.refresh(), detail.refresh()])
  }
  // The detail fetch is fresher than the list, and also covers an agent the list's filters hide.
  const selected: HarnessSession | null = detail.data ?? list.data?.find((s) => s.id === selectedId) ?? null

  const collapsed = sidebar.collapsed
  return (
    <div className="flex h-full min-h-0 w-full max-w-full overflow-hidden">
      <aside
        aria-label="Agents"
        style={{ '--wb-w': collapsed ? '72px' : `${sidebar.width}px` } as React.CSSProperties}
        className={`${selectedId ? 'hidden md:flex' : 'flex'} flex-col w-full md:w-[var(--wb-w)] md:border-r md:border-white/10 min-h-0 shrink-0 ${sidebar.dragging ? '' : 'md:transition-[width] md:duration-200'}`}
      >
        {collapsed ? (
          <CollapsedRail list={list.data ?? []} selectedId={selectedId} onSelect={select} onNew={() => setCreating(true)} onExpand={sidebar.toggle} />
        ) : (
          <>
            <div className="flex items-center gap-2 px-4 pt-4 pb-3">
              <h1 className="text-xl font-semibold tracking-tight text-neutral-50 mr-auto" style={{ fontFamily: 'var(--font-heading)' }}>Workbench</h1>
              <button
                type="button"
                onClick={() => void list.refresh()}
                className="hq-icon-btn"
                aria-label="Refresh the list"
              >
                <RefreshCw className="w-4 h-4" />
              </button>
              <span className="hidden md:contents">
                <button type="button" onClick={sidebar.toggle} className="hq-icon-btn" aria-label="Collapse the sidebar">
                  <PanelLeftClose className="w-4 h-4" />
                </button>
              </span>
            </div>
            <div className="px-4 pb-3">
              <button type="button" onClick={() => setCreating(true)} className="hq-btn-primary w-full">
                <Plus className="w-4 h-4" />
                New agent
              </button>
            </div>
            {task && (
              <button
                type="button"
                onClick={() => void navigate({ search: (prev) => ({ ...prev, task: undefined }), replace: true })}
                className="px-4 min-h-11 text-left text-xs text-neutral-400 hover:text-white border-y border-white/5"
              >
                Showing one task's agents. Show all.
              </button>
            )}
            <AgentList list={list} selectedId={selectedId} showArchived={showArchived} onShowArchived={setShowArchived} onSelect={select} onNew={() => setCreating(true)} />
          </>
        )}
      </aside>
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the sidebar"
        aria-valuemin={SIDEBAR_MIN}
        aria-valuemax={SIDEBAR_MAX}
        aria-valuenow={collapsed ? 72 : sidebar.width}
        tabIndex={0}
        onPointerDown={collapsed ? undefined : sidebar.startDrag}
        onDoubleClick={sidebar.reset}
        onKeyDown={sidebar.onKeyDown}
        className={`hq-resize-handle hidden md:block ${sidebar.dragging ? 'is-dragging' : ''} ${collapsed ? 'is-disabled' : ''}`}
      />
      <main className={`${selectedId ? 'flex' : 'hidden md:flex'} flex-1 min-w-0 min-h-0 flex-col`}>
        <div className="flex flex-col flex-1 min-h-0 min-w-0 overflow-hidden">
          <div className="mx-auto flex flex-1 min-h-0 min-w-0 w-full max-w-[1500px]">
            <div className="flex flex-col flex-1 min-h-0 min-w-0 max-w-[1100px] mx-auto">
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
                  Pick an agent to see what it is doing.
                </div>
              )}
            </div>
            {selected && (
              <div className="hidden 2xl:flex">
                <SessionInspector session={selected} />
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
  onSelect: (id: string) => void
  onNew: () => void
}

function SectionTitle({ children, count }: { children: string; count: number }) {
  return (
    <h2 className="flex items-center gap-2 px-2 pt-4 pb-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400">
      {children}
      <span className="px-1.5 rounded-full bg-white/10 text-[10px] text-neutral-300">{count}</span>
    </h2>
  )
}

function AgentList({ list, selectedId, showArchived, onShowArchived, onSelect, onNew }: ListProps) {
  const [archiveError, setArchiveError] = useState<string | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [confirmAll, setConfirmAll] = useState(false)
  const groups = useMemo(() => groupSessions(list.data ?? [], showArchived), [list.data, showArchived])

  const archive = async (ids: string[], archived: boolean) => {
    setBusyId(ids.length === 1 ? ids[0] : 'all')
    setArchiveError(null)
    try {
      await Promise.all(ids.map((id) => globalSessionsApi.archive(id, archived)))
    } catch (err) {
      setArchiveError(err instanceof Error ? err.message : 'Could not update the list.')
    } finally {
      setBusyId(null)
      setConfirmAll(false)
      await list.refresh()
    }
  }

  if (list.loading) {
    return (
      <div className="flex items-center gap-2 p-4 text-xs font-mono text-neutral-500" role="status">
        <Loader2 className="w-4 h-4 animate-spin" />
        Loading your agents
      </div>
    )
  }
  if (!list.data) {
    return (
      <ErrorText className="p-4 text-xs">{list.error ?? 'Could not load your agents.'}</ErrorText>
    )
  }

  const row = (s: HarnessSession, canArchive = false) => (
    <SessionRow key={s.id} session={s} selected={s.id === selectedId} onSelect={() => onSelect(s.id)} onArchive={canArchive ? (a) => void archive([s.id], a) : undefined} busy={busyId === s.id || busyId === 'all'} />
  )
  const pastIds = archivable(list.data).map((s) => s.id)
  const empty = groups.needsYou.length + groups.working.length + groups.past.length === 0

  return (
    <div className="flex-1 overflow-y-auto overscroll-contain min-h-0 pb-4 px-2">
      {list.error && (
        <ErrorText className="px-3 py-1.5">Could not refresh: {list.error}</ErrorText>
      )}
      {archiveError && (
        <ErrorText className="px-3 py-1.5">{archiveError}</ErrorText>
      )}
      {empty && (
        <div className="flex flex-col items-center gap-3 p-6 text-center">
          <Terminal className="w-6 h-6 text-neutral-500" />
          <p className="text-xs font-mono text-neutral-400">No agents yet. Start one and it will show up here.</p>
          <button type="button" onClick={onNew} className={NEW_AGENT_BUTTON_CLASS}>
            <Plus className="w-3.5 h-3.5" />
            New agent
          </button>
        </div>
      )}
      {groups.needsYou.length > 0 && (
        <section aria-label="Needs you">
          <SectionTitle count={groups.needsYou.length}>Needs you</SectionTitle>
          <ul>{groups.needsYou.map((s) => row(s))}</ul>
        </section>
      )}
      {groups.working.length > 0 && (
        <section aria-label="Working and ready">
          <SectionTitle count={groups.working.length}>Working and ready</SectionTitle>
          <ul>{groups.working.map((s) => row(s))}</ul>
        </section>
      )}
      {groups.past.length > 0 && (
        <details className="mt-2">
          <summary className="flex items-center gap-2 px-3 min-h-11 cursor-pointer text-[11px] font-mono font-semibold uppercase tracking-wider text-neutral-400 select-none">
            Past agents
            <span className="px-1.5 rounded-full bg-white/10 text-[10px] text-neutral-300">{groups.past.length}</span>
          </summary>
          {pastIds.length > 0 && (
            <div className="px-3 pb-2">
              {confirmAll ? (
                <div className="flex items-center gap-2" role="group" aria-label="Confirm archive all">
                  <button type="button" disabled={busyId !== null} onClick={() => void archive(pastIds, true)} className="h-11 sm:h-9 px-3 rounded border border-white/10 text-[11px] font-mono text-neutral-200 hover:bg-white/10 disabled:opacity-50">
                    Yes, archive {pastIds.length}
                  </button>
                  <button type="button" onClick={() => setConfirmAll(false)} className="h-11 sm:h-9 px-3 rounded border border-white/10 text-[11px] font-mono text-neutral-400 hover:bg-white/10">
                    Cancel
                  </button>
                </div>
              ) : (
                <button type="button" onClick={() => setConfirmAll(true)} className="flex items-center gap-1.5 h-11 sm:h-9 px-3 rounded border border-white/10 text-[11px] font-mono text-neutral-300 hover:text-white hover:bg-white/10">
                  <Archive className="w-3.5 h-3.5" />
                  Archive all past agents
                </button>
              )}
            </div>
          )}
          <ul>{groups.past.map((s) => row(s, true))}</ul>
        </details>
      )}
      <label className="flex items-center gap-2 px-3 min-h-11 text-[11px] font-mono text-neutral-400 cursor-pointer">
        <input type="checkbox" checked={showArchived} onChange={(e) => onShowArchived(e.target.checked)} className="h-4 w-4" style={{ accentColor: 'var(--accent-green)' }} />
        Show archived
      </label>
    </div>
  )
}

function DetailPlaceholder({ error, loading, onBack }: { error: string | null; loading: boolean; onBack: () => void }) {
  if (loading) {
    return (
      <div className="m-auto flex items-center gap-2 text-xs font-mono text-neutral-500" role="status">
        <Loader2 className="w-4 h-4 animate-spin" />
        Loading this agent
      </div>
    )
  }
  return (
    <div className="m-auto flex flex-col items-center gap-2 p-6 text-center text-xs font-mono">
      <ErrorText className="text-xs">{error ?? 'We could not find that agent.'}</ErrorText>
      <button type="button" onClick={onBack} className="h-9 px-3 rounded border border-white/10 text-neutral-300 hover:bg-white/10">
        Back to the Workbench
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
}

const initials = (title: string) =>
  title
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]?.toUpperCase() ?? '')
    .join('')

/** The sidebar folded down to one avatar per agent, with a status dot, so nothing that needs you is hidden. */
function CollapsedRail({ list, selectedId, onSelect, onNew, onExpand }: RailProps) {
  const visible = list.filter((s) => !s.archived).slice(0, 14)
  return (
    <div className="flex flex-col items-center gap-2.5 py-4 overflow-y-auto min-h-0">
      <button type="button" onClick={onExpand} className="hq-icon-btn" aria-label="Expand the sidebar">
        <PanelLeftOpen className="w-4 h-4" />
      </button>
      <button type="button" onClick={onNew} className="hq-btn-primary !h-11 !w-11 !p-0" aria-label="New agent">
        <Plus className="w-4 h-4" />
      </button>
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
