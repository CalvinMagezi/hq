import { createFileRoute } from '@tanstack/react-router'
import { Archive, Loader2, Plus, RefreshCw, Terminal } from 'lucide-react'
import { useMemo, useState } from 'react'
import { NewAgentDialog } from '~/components/sessions/NewAgentDialog'
import { SessionDetail } from '~/components/sessions/SessionDetail'
import { SessionRow } from '~/components/sessions/SessionRow'
import { usePolled } from '~/components/sessions/usePolled'
import { globalSessionsApi, type HarnessSession } from '~/lib/sessionsApi'
import { archivable, groupSessions } from '~/lib/workbench'

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

const NEW_AGENT_BUTTON_CLASS =
  'flex items-center justify-center gap-1.5 h-11 sm:h-9 px-3 rounded-lg border border-white/10 bg-white/10 text-xs font-mono font-semibold text-white hover:bg-white/15 shrink-0'

function WorkbenchPage() {
  const { id: selectedId, task } = Route.useSearch()
  const navigate = Route.useNavigate()
  const [showArchived, setShowArchived] = useState(false)
  const [creating, setCreating] = useState(false)
  const filters = useMemo(() => ({ ...(task ? { task_id: task } : {}), ...(showArchived ? { include_archived: true } : {}) }), [task, showArchived])
  const list = usePolled(JSON.stringify(filters), () => globalSessionsApi.list(filters), LIST_POLL_MS)
  const detail = usePolled(selectedId ?? '', () => globalSessionsApi.get(selectedId ?? ''), DETAIL_POLL_MS, Boolean(selectedId))

  const select = (id: string | undefined) => void navigate({ search: (prev) => ({ ...prev, id }), replace: true })
  const refreshAll = () => void Promise.all([list.refresh(), detail.refresh()])
  // The detail fetch is fresher than the list, and also covers an agent the list's filters hide.
  const selected: HarnessSession | null = detail.data ?? list.data?.find((s) => s.id === selectedId) ?? null

  return (
    <div className="flex h-full min-h-0 w-full max-w-full overflow-hidden bg-neutral-950">
      <aside className={`${selectedId ? 'hidden md:flex' : 'flex'} flex-col w-full md:w-96 md:border-r border-white/10 min-h-0 shrink-0`}>
        <div className="flex items-center gap-2 px-3 py-2 border-b border-white/10">
          <h1 className="text-sm font-mono font-semibold text-neutral-100 mr-auto">Workbench</h1>
          <button
            type="button"
            onClick={() => void list.refresh()}
            className="flex items-center justify-center h-11 w-11 sm:h-9 sm:w-9 shrink-0 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10"
            aria-label="Refresh the list"
          >
            <RefreshCw className="w-3.5 h-3.5" />
          </button>
          <button type="button" onClick={() => setCreating(true)} className={NEW_AGENT_BUTTON_CLASS}>
            <Plus className="w-3.5 h-3.5" />
            New agent
          </button>
        </div>
        {task && (
          <button
            type="button"
            onClick={() => void navigate({ search: (prev) => ({ ...prev, task: undefined }), replace: true })}
            className="px-3 min-h-11 text-left text-[11px] font-mono text-neutral-400 hover:text-white border-b border-white/5"
          >
            Showing one task's agents. Show all.
          </button>
        )}
        <AgentList list={list} selectedId={selectedId} showArchived={showArchived} onShowArchived={setShowArchived} onSelect={select} onNew={() => setCreating(true)} />
      </aside>
      <main className={`${selectedId ? 'flex' : 'hidden md:flex'} flex-1 min-w-0 min-h-0 flex-col`}>
        {selectedId ? (
          selected ? (
            <SessionDetail key={selected.id} session={selected} onBack={() => select(undefined)} onChanged={refreshAll} />
          ) : (
            <DetailPlaceholder error={detail.error} loading={detail.loading} onBack={() => select(undefined)} />
          )
        ) : (
          <div className="m-auto flex flex-col items-center gap-2 text-neutral-500 text-xs font-mono p-6 text-center">
            <Terminal className="w-6 h-6" />
            Pick an agent to see what it is doing.
          </div>
        )}
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
    <h2 className="flex items-center gap-2 px-3 pt-3 pb-1 text-[11px] font-mono font-semibold uppercase tracking-wider text-neutral-400">
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
      <p role="alert" className="p-4 text-xs font-mono text-rose-400">
        {list.error ?? 'Could not load your agents.'}
      </p>
    )
  }

  const row = (s: HarnessSession, canArchive = false) => (
    <SessionRow key={s.id} session={s} selected={s.id === selectedId} onSelect={() => onSelect(s.id)} onArchive={canArchive ? (a) => void archive([s.id], a) : undefined} busy={busyId === s.id || busyId === 'all'} />
  )
  const pastIds = archivable(list.data).map((s) => s.id)
  const empty = groups.needsYou.length + groups.working.length + groups.past.length === 0

  return (
    <div className="flex-1 overflow-y-auto overscroll-contain min-h-0 pb-4">
      {list.error && (
        <p role="alert" className="px-3 py-1.5 text-[11px] font-mono text-rose-400">
          Could not refresh: {list.error}
        </p>
      )}
      {archiveError && (
        <p role="alert" className="px-3 py-1.5 text-[11px] font-mono text-rose-400">
          {archiveError}
        </p>
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
        <input type="checkbox" checked={showArchived} onChange={(e) => onShowArchived(e.target.checked)} className="h-4 w-4 accent-emerald-400" />
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
      <p role="alert" className="text-rose-400">
        {error ?? 'We could not find that agent.'}
      </p>
      <button type="button" onClick={onBack} className="h-9 px-3 rounded border border-white/10 text-neutral-300 hover:bg-white/10">
        Back to the Workbench
      </button>
    </div>
  )
}
