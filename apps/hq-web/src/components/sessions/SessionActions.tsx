import { useState } from 'react'
import { Link } from '@tanstack/react-router'
import { Archive, ArchiveRestore, Check, Eye, Loader2, Pencil, Play, Square, X } from 'lucide-react'
import { useThreadStore } from '~/store/threadStore'
import { refreshThreads } from '~/lib/threadApi'
import { globalSessionsApi, type HarnessSession } from '~/lib/sessionsApi'
import { sessionTitle } from '~/lib/workbench'
import { ErrorText } from './ErrorText'

export const ACTION_BUTTON_CLASS =
  'flex items-center justify-center gap-1.5 h-11 sm:h-9 px-3 rounded border border-white/10 text-[11px] text-neutral-200 hover:text-white hover:bg-white/10 disabled:opacity-50 disabled:hover:bg-transparent shrink-0'

export interface ActionState {
  busy: boolean
  error: string | null
  /** Resolves true on success. Stays busy until the follow-up refresh has settled. */
  run: (action: () => Promise<unknown>, fallback: string) => Promise<boolean>
}

/** One action area: only one request at a time, and no new one until the refresh after it lands. */
export function useAction(onDone: () => void | Promise<void>): ActionState {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = async (action: () => Promise<unknown>, fallback: string) => {
    if (busy) return false
    setBusy(true)
    setError(null)
    try {
      await action()
      await onDone()
      return true
    } catch (err) {
      setError(err instanceof Error ? err.message : fallback)
      return false
    } finally {
      setBusy(false)
    }
  }
  return { busy, error, run }
}

interface ActionProps {
  session: HarnessSession
  action: ActionState
}

const Spinner = <Loader2 className="w-3.5 h-3.5 animate-spin" />

/** The agent's name, editable in place. The field starts empty when no custom name is set. */
export function RenameTitle({ session: s, action }: ActionProps) {
  const [editing, setEditing] = useState(false)
  const [value, setValue] = useState('')
  const title = sessionTitle(s)

  if (!editing) {
    return (
      <div className="flex items-center gap-1 min-w-0">
        <h2 className="text-sm text-neutral-100 truncate min-w-0">{title}</h2>
        <button
          type="button"
          onClick={() => {
            setValue(s.label ?? '')
            setEditing(true)
          }}
          disabled={action.busy}
          aria-label="Rename"
          title="Rename"
          className="flex items-center justify-center h-11 w-11 sm:h-9 sm:w-9 shrink-0 rounded text-neutral-500 hover:text-white hover:bg-white/10 disabled:opacity-50"
        >
          <Pencil className="w-3.5 h-3.5" />
        </button>
      </div>
    )
  }
  const save = async () => {
    if (await action.run(() => globalSessionsApi.rename(s.id, value.trim()), 'Could not rename this agent.')) setEditing(false)
  }
  return (
    <div className="flex items-center gap-1.5 min-w-0 flex-1">
      <input
        autoFocus
        value={value}
        placeholder={title}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') void save()
          if (e.key === 'Escape') setEditing(false)
        }}
        aria-label="Agent name"
        maxLength={80}
        className="flex-1 min-w-0 h-11 sm:h-9 px-3 rounded-lg text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400"
      />
      <button type="button" onClick={() => void save()} disabled={action.busy} aria-label="Save name" className={ACTION_BUTTON_CLASS}>
        {action.busy ? Spinner : <Check className="w-3.5 h-3.5" />}
      </button>
      <button type="button" onClick={() => setEditing(false)} disabled={action.busy} aria-label="Cancel rename" className={ACTION_BUTTON_CLASS}>
        <X className="w-3.5 h-3.5" />
      </button>
    </div>
  )
}

/** Stop asks twice, since it ends the agent's work. */
export function StopButton({ session: s, action }: ActionProps) {
  const [confirming, setConfirming] = useState(false)
  if (!confirming) {
    return (
      <button type="button" disabled={action.busy} onClick={() => setConfirming(true)} className={ACTION_BUTTON_CLASS}>
        <Square className="w-3.5 h-3.5" />
        Stop
      </button>
    )
  }
  return (
    <div className="flex items-center gap-1.5" role="group" aria-label="Confirm stop">
      <button
        type="button"
        disabled={action.busy}
        onClick={async () => {
          if (await action.run(() => globalSessionsApi.stop(s.id), 'Could not stop this agent.')) setConfirming(false)
        }}
        className={ACTION_BUTTON_CLASS}
        style={{ color: 'var(--accent-red)' }}
      >
        {action.busy ? Spinner : <Square className="w-3.5 h-3.5" />}
        Yes, stop it
      </button>
      <button type="button" disabled={action.busy} onClick={() => setConfirming(false)} className={ACTION_BUTTON_CLASS}>
        Keep going
      </button>
    </div>
  )
}

export function ResumeButton({ session: s, action }: ActionProps) {
  return (
    <button
      type="button"
      disabled={action.busy}
      onClick={() => void action.run(() => globalSessionsApi.resume(s.id), 'Could not resume this agent.')}
      className={`${ACTION_BUTTON_CLASS} sm:h-11 px-5 text-xs bg-white/10`}
    >
      {action.busy ? Spinner : <Play className="w-3.5 h-3.5" />}
      Resume
    </button>
  )
}

export function ArchiveButton({ session: s, action }: ActionProps) {
  const Icon = s.archived ? ArchiveRestore : Archive
  return (
    <button
      type="button"
      disabled={action.busy}
      onClick={() => void action.run(() => globalSessionsApi.archive(s.id, !s.archived), 'Could not update this agent.')}
      className={ACTION_BUTTON_CLASS}
    >
      {action.busy ? Spinner : <Icon className="w-3.5 h-3.5" />}
      {s.archived ? 'Restore' : 'Archive'}
    </button>
  )
}

/** Starts a chat that follows the agent, or opens the chat that already does. */
export function FollowInChat({ session: s, action }: ActionProps) {
  const [threadId, setThreadId] = useState(s.owner_thread)
  const follow = () =>
    action.run(async () => {
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
    <button
      type="button"
      onClick={() => void follow()}
      disabled={action.busy}
      className={ACTION_BUTTON_CLASS}
      title="Start a chat that follows this agent. HQ only watches and will not answer for it."
    >
      {action.busy ? Spinner : <Eye className="w-3.5 h-3.5" />}
      Follow in chat
    </button>
  )
}

export function ActionError({ error }: { error: string | null }) {
  return error ? <ErrorText>{error}</ErrorText> : null
}
