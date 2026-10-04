import { createFileRoute } from '@tanstack/react-router'
import { Loader2, RefreshCw, Terminal } from 'lucide-react'
import { useMemo, useState } from 'react'
import { SessionDetail } from '~/components/sessions/SessionDetail'
import { SessionRow } from '~/components/sessions/SessionRow'
import { usePolled } from '~/components/sessions/usePolled'
import { globalSessionsApi, type HarnessSession, type SessionStatus } from '~/lib/sessionsApi'

const LIST_POLL_MS = 8_000
const DETAIL_POLL_MS = 5_000
const STATUSES: SessionStatus[] = ['running', 'exited', 'stopped', 'orphaned']
const SELECT_CLASS =
  'h-11 sm:h-9 min-w-0 flex-1 sm:flex-none px-2 rounded-lg text-xs font-mono text-neutral-200 bg-black/30 border border-white/10 focus:outline-none focus:ring-1 focus:ring-emerald-400'

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
  component: SessionsPage,
})

function SessionsPage() {
  const { id: selectedId, task } = Route.useSearch()
  const navigate = Route.useNavigate()
  const [status, setStatus] = useState<SessionStatus | ''>('')
  const [host, setHost] = useState('')
  const filters = useMemo(
    () => ({ ...(status ? { status } : {}), ...(host ? { host } : {}), ...(task ? { task_id: task } : {}) }),
    [status, host, task]
  )
  const list = usePolled(JSON.stringify(filters), () => globalSessionsApi.list(filters), LIST_POLL_MS)
  const detail = usePolled(selectedId ?? '', () => globalSessionsApi.get(selectedId ?? ''), DETAIL_POLL_MS, Boolean(selectedId))

  const select = (id: string | undefined) => void navigate({ search: (prev) => ({ ...prev, id }), replace: true })
  const hosts = useMemo(() => [...new Set((list.data ?? []).map((s) => s.host))].sort(), [list.data])
  // The detail fetch is fresher than the list, and also covers a session the list's filters hide.
  const selected: HarnessSession | null = detail.data ?? list.data?.find((s) => s.id === selectedId) ?? null

  return (
    <div className="flex h-full min-h-0 w-full max-w-full overflow-hidden bg-neutral-950">
      <aside className={`${selectedId ? 'hidden md:flex' : 'flex'} flex-col w-full md:w-96 md:border-r border-white/10 min-h-0 shrink-0`}>
        <div className="flex flex-wrap items-center gap-2 px-3 py-2 border-b border-white/10">
          <h1 className="text-sm font-mono font-semibold text-neutral-100 sm:mr-auto basis-full sm:basis-auto">Sessions</h1>
          <select value={status} onChange={(e) => setStatus(e.target.value as SessionStatus | '')} aria-label="Filter by status" className={SELECT_CLASS}>
            <option value="">All statuses</option>
            {STATUSES.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
          <select value={host} onChange={(e) => setHost(e.target.value)} aria-label="Filter by host" className={SELECT_CLASS}>
            <option value="">All hosts</option>
            {hosts.map((h) => (
              <option key={h} value={h}>
                {h}
              </option>
            ))}
          </select>
          <button
            type="button"
            onClick={() => void list.refresh()}
            className="flex items-center justify-center h-11 w-11 sm:h-9 sm:w-9 shrink-0 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10"
            aria-label="Refresh sessions"
          >
            <RefreshCw className="w-3.5 h-3.5" />
          </button>
        </div>
        {task && (
          <button
            type="button"
            onClick={() => void navigate({ search: (prev) => ({ ...prev, task: undefined }), replace: true })}
            className="px-3 py-1.5 text-left text-[11px] font-mono text-neutral-400 hover:text-white border-b border-white/5"
          >
            Showing one task's sessions. Show all.
          </button>
        )}
        <SessionList list={list} selectedId={selectedId} onSelect={select} />
      </aside>
      <main className={`${selectedId ? 'flex' : 'hidden md:flex'} flex-1 min-w-0 min-h-0 flex-col`}>
        {selectedId ? (
          selected ? (
            <SessionDetail key={selected.id} session={selected} onBack={() => select(undefined)} onChanged={() => void Promise.all([list.refresh(), detail.refresh()])} />
          ) : (
            <DetailPlaceholder error={detail.error} loading={detail.loading} onBack={() => select(undefined)} />
          )
        ) : (
          <div className="m-auto flex flex-col items-center gap-2 text-neutral-500 text-xs font-mono p-6 text-center">
            <Terminal className="w-6 h-6" />
            Pick a session to see its terminal.
          </div>
        )}
      </main>
    </div>
  )
}

interface ListProps {
  list: ReturnType<typeof usePolled<HarnessSession[]>>
  selectedId: string | undefined
  onSelect: (id: string) => void
}

function SessionList({ list, selectedId, onSelect }: ListProps) {
  if (list.loading) {
    return (
      <div className="flex items-center gap-2 p-4 text-xs font-mono text-neutral-500" role="status">
        <Loader2 className="w-4 h-4 animate-spin" />
        Loading sessions
      </div>
    )
  }
  if (!list.data) {
    return (
      <p role="alert" className="p-4 text-xs font-mono text-rose-400">
        {list.error ?? 'Could not load sessions.'}
      </p>
    )
  }
  return (
    <div className="flex-1 overflow-y-auto overscroll-contain min-h-0">
      {list.error && (
        <p role="alert" className="px-3 py-1.5 text-[11px] font-mono text-rose-400">
          Refresh failed: {list.error}
        </p>
      )}
      {list.data.length === 0 ? (
        <p className="p-4 text-xs font-mono text-neutral-500">No sessions match. Start one with harness_session_spawn.</p>
      ) : (
        <ul aria-label="Coding-agent sessions">
          {list.data.map((s) => (
            <SessionRow key={s.id} session={s} selected={s.id === selectedId} onSelect={() => onSelect(s.id)} />
          ))}
        </ul>
      )}
    </div>
  )
}

function DetailPlaceholder({ error, loading, onBack }: { error: string | null; loading: boolean; onBack: () => void }) {
  if (loading) {
    return (
      <div className="m-auto flex items-center gap-2 text-xs font-mono text-neutral-500" role="status">
        <Loader2 className="w-4 h-4 animate-spin" />
        Loading session
      </div>
    )
  }
  return (
    <div className="m-auto flex flex-col items-center gap-2 p-6 text-center text-xs font-mono">
      <p role="alert" className="text-rose-400">
        {error ?? 'Session not found.'}
      </p>
      <button type="button" onClick={onBack} className="h-9 px-3 rounded border border-white/10 text-neutral-300 hover:bg-white/10">
        Back to sessions
      </button>
    </div>
  )
}
