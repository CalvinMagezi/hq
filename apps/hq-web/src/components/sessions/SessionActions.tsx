import { useState } from 'react'
import { Link } from '@tanstack/react-router'
import { Archive, ArchiveRestore, Check, Eye, Loader2, Pencil, Play, Square, X } from 'lucide-react'
import { useThreadStore } from '~/store/threadStore'
import { refreshThreads } from '~/lib/threadApi'
import { globalSessionsApi, type HarnessSession } from '~/lib/sessionsApi'
import { sessionTitle } from '~/lib/workbench'

export const ACTION_BUTTON_CLASS =
  'flex items-center justify-center gap-1.5 h-11 sm:h-9 px-3 rounded border border-white/10 text-[11px] font-mono text-neutral-200 hover:text-white hover:bg-white/10 disabled:opacity-50 disabled:hover:bg-transparent shrink-0'

/** Runs one server action, tracking busy and a plain-language error. */
export function useAction(onDone: () => void) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = async (action: () => Promise<unknown>, fallback: string) => {
    setBusy(true)
    setError(null)
    try {
      await action()
      onDone()
    } catch (err) {
      setError(err instanceof Error ? err.message : fallback)
    } finally {
      setBusy(false)
    }
  }
  return { busy, error, run }
}

function ActionError({ error }: { error: string | null }) {
  if (!error) return null
  return (
    <p role="alert" className="text-[11px] font-mono text-rose-400 break-words">
      {error}
    </p>
  )
}

interface ActionProps {
  session: HarnessSession
  onChanged: () => void
}

/** The agent's name, editable in place. */
export function RenameTitle({ session: s, onChanged }: ActionProps) {
  const [editing, setEditing] = useState(false)
  const [value, setValue] = useState('')
  const { busy, error, run } = useAction(() => {
    setEditing(false)
    onChanged()
  })
  const title = sessionTitle(s)

  if (!editing) {
    return (
      <div className="flex items-center gap-1 min-w-0">
        <h2 className="text-sm font-mono text-neutral-100 truncate min-w-0" title={s.cwd}>
          {title}
        </h2>
        <button
          type="button"
          onClick={() => {
            setValue(s.label || title)
            setEditing(true)
          }}
          aria-label="Rename"
          title="Rename"
          className="flex items-center justify-center h-11 w-11 sm:h-9 sm:w-9 shrink-0 rounded text-neutral-500 hover:text-white hover:bg-white/10"
        >
          <Pencil className="w-3.5 h-3.5" />
        </button>
      </div>
    )
  }
  const save = () => void run(() => globalSessionsApi.rename(s.id, value.trim()), 'Could not rename this agent.')
  return (
    <div className="space-y-1 min-w-0 flex-1">
      <div className="flex items-center gap-1.5">
        <input
          autoFocus
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') save()
            if (e.key === 'Escape') setEditing(false)
          }}
          aria-label="Agent name"
          maxLength={80}
          className="flex-1 min-w-0 h-11 sm:h-9 px-3 rounded-lg text-xs font-mono text-neutral-200 bg-black/30 border border-white/10 focus:outline-none focus:ring-1 focus:ring-emerald-400"
        />
        <button type="button" onClick={save} disabled={busy} aria-label="Save name" className={ACTION_BUTTON_CLASS}>
          {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Check className="w-3.5 h-3.5" />}
        </button>
        <button type="button" onClick={() => setEditing(false)} aria-label="Cancel rename" className={ACTION_BUTTON_CLASS}>
          <X className="w-3.5 h-3.5" />
        </button>
      </div>
      <ActionError error={error} />
    </div>
  )
}

/** Stop asks twice, since it ends the agent's work. */
export function StopButton({ session: s, onChanged }: ActionProps) {
  const [confirming, setConfirming] = useState(false)
  const { busy, error, run } = useAction(() => {
    setConfirming(false)
    onChanged()
  })
  return (
    <div className="flex flex-col gap-1">
      {confirming ? (
        <div className="flex items-center gap-1.5" role="group" aria-label="Confirm stop">
          <button
            type="button"
            disabled={busy}
            onClick={() => void run(() => globalSessionsApi.stop(s.id), 'Could not stop this agent.')}
            className={`${ACTION_BUTTON_CLASS} border-rose-400/40 text-rose-300 hover:text-rose-200 hover:bg-rose-500/10`}
          >
            {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Square className="w-3.5 h-3.5" />}
            Yes, stop it
          </button>
          <button type="button" onClick={() => setConfirming(false)} className={ACTION_BUTTON_CLASS}>
            Keep going
          </button>
        </div>
      ) : (
        <button type="button" onClick={() => setConfirming(true)} className={ACTION_BUTTON_CLASS}>
          <Square className="w-3.5 h-3.5" />
          Stop
        </button>
      )}
      <ActionError error={error} />
    </div>
  )
}

export function ResumeButton({ session: s, onChanged, prominent }: ActionProps & { prominent?: boolean }) {
  const { busy, error, run } = useAction(onChanged)
  return (
    <div className="flex flex-col gap-1">
      <button
        type="button"
        disabled={busy}
        onClick={() => void run(() => globalSessionsApi.resume(s.id), 'Could not resume this agent.')}
        className={prominent ? `${ACTION_BUTTON_CLASS} sm:h-11 px-5 text-xs bg-white/10` : ACTION_BUTTON_CLASS}
      >
        {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Play className="w-3.5 h-3.5" />}
        Resume
      </button>
      <ActionError error={error} />
    </div>
  )
}

export function ArchiveButton({ session: s, onChanged }: ActionProps) {
  const { busy, error, run } = useAction(onChanged)
  const Icon = s.archived ? ArchiveRestore : Archive
  return (
    <div className="flex flex-col gap-1">
      <button
        type="button"
        disabled={busy}
        onClick={() => void run(() => globalSessionsApi.archive(s.id, !s.archived), 'Could not update this agent.')}
        className={ACTION_BUTTON_CLASS}
      >
        {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Icon className="w-3.5 h-3.5" />}
        {s.archived ? 'Restore' : 'Archive'}
      </button>
      <ActionError error={error} />
    </div>
  )
}

/** Starts a chat that follows the agent, or opens the chat that already does. */
export function FollowInChat({ session: s, onChanged }: ActionProps) {
  const [threadId, setThreadId] = useState(s.owner_thread)
  const { busy, error, run } = useAction(onChanged)
  const follow = () =>
    run(async () => {
      const result = await globalSessionsApi.adopt(s.id)
      setThreadId(result.thread_id)
      await refreshThreads()
      useThreadStore.getState().setActiveThread(result.thread_id)
    }, 'Could not start the chat.')

  if (threadId) {
    return (
      <Link
        to="/chat"
        onClick={() => useThreadStore.getState().setActiveThread(threadId)}
        className={ACTION_BUTTON_CLASS}
        title="A chat is following this agent. Open it."
      >
        <Eye className="w-3.5 h-3.5" />
        Open chat
      </Link>
    )
  }
  return (
    <div className="flex flex-col gap-1">
      <button
        type="button"
        onClick={() => void follow()}
        disabled={busy}
        className={ACTION_BUTTON_CLASS}
        title="Start a chat that follows this agent. HQ only watches and will not answer for it."
      >
        {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Eye className="w-3.5 h-3.5" />}
        Follow in chat
      </button>
      <ActionError error={error} />
    </div>
  )
}
