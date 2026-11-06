import { Dialog } from '../ui/Dialog'
import { t } from '~/strings'
import { useState } from 'react'
import { Link } from '@tanstack/react-router'
import { Archive, ArchiveRestore, Check, Eye, Loader2, Pencil, Play, Square, X } from 'lucide-react'
import { useThreadStore } from '~/store/threadStore'
import { refreshThreads } from '~/lib/threadApi'
import { globalSessionsApi, type HarnessSession } from '~/lib/sessionsApi'
import { sessionTitle } from '~/lib/workbench'
import { sharedConnectionNow } from '~/lib/connection'
import { unreachableOr } from '~/lib/failureWords'
import { ErrorText } from './ErrorText'

export const ACTION_BUTTON_CLASS = 'a-btn secondary'

export interface ActionState {
  busy: boolean
  error: string | null
  /** Resolves true on success. Stays busy until the follow-up refresh has settled. */
  run: (action: () => Promise<unknown>, fallback: string | (() => string)) => Promise<boolean>
}

/** One action area: only one request at a time, and no new one until the refresh after it lands. */
export function useAction(onDone: () => void | Promise<void>): ActionState {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = async (action: () => Promise<unknown>, fallback: string | (() => string)) => {
    if (busy) return false
    setBusy(true)
    setError(null)
    try {
      await action()
      await onDone()
      return true
    } catch (err) {
      setError(err instanceof Error && !(err instanceof TypeError) ? err.message : typeof fallback === 'function' ? fallback() : fallback)
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

const Spinner = <Loader2 className="a-ic animate-spin" aria-hidden="true" />

/** The agent's name, editable in place. The field starts empty when no custom name is set. */
export function RenameTitle({ session: s, action }: ActionProps) {
  const [editing, setEditing] = useState(false)
  const [value, setValue] = useState('')
  const title = sessionTitle(s)

  if (!editing) {
    return (
      <button
        type="button"
        onClick={() => {
          setValue(s.label ?? '')
          setEditing(true)
        }}
        disabled={action.busy}
        className={ACTION_BUTTON_CLASS}
      >
        <Pencil className="a-ic" aria-hidden="true" />
        {t('helpers_rename')}
      </button>
    )
  }
  const save = async () => {
    if (await action.run(() => globalSessionsApi.rename(s.id, value.trim()), t('helpers_rename_failed', { helper: title }))) setEditing(false)
  }
  return (
    <div className="a-field">
      <input
        autoFocus
        value={value}
        placeholder={title}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') void save()
          if (e.key === 'Escape') setEditing(false)
        }}
        aria-label={t('helpers_rename_label')}
        maxLength={80}
        className="a-input"
      />
      <div className="a-btns">
        <button type="button" onClick={() => setEditing(false)} disabled={action.busy} aria-label={t('helpers_rename_cancel')} className={ACTION_BUTTON_CLASS}>
          <X className="a-ic" aria-hidden="true" />
          {t('helpers_new_cancel')}
        </button>
        <button type="button" onClick={() => void save()} disabled={action.busy} aria-label={t('helpers_rename_save')} className="a-btn">
          {action.busy ? Spinner : <Check className="a-ic" aria-hidden="true" />}
          {t('helpers_rename_save')}
        </button>
      </div>
    </div>
  )
}

/** Stop asks first, since it ends the agent's work. */
export function StopButton({ session: s, action }: ActionProps) {
  const [confirming, setConfirming] = useState(false)
  const helper = s.label || s.id
  return (
    <>
      <button type="button" disabled={action.busy} onClick={() => setConfirming(true)} className="a-btn quiet danger">
        <Square className="a-ic" aria-hidden="true" />
        {t('helpers_stop_named', { helper })}
      </button>
      {confirming && (
        <Dialog
          kind="red"
          title={t('stop_helper_title', { helper })}
          body={t('stop_helper_body')}
          keepLabel={t('stop_helper_keep')}
          actLabel={t('stop_helper_act', { helper })}
          busyLabel={t('confirm_busy_stopping')}
          busy={action.busy}
          onKeep={() => setConfirming(false)}
          onAct={async () => {
            if (await action.run(() => globalSessionsApi.stop(s.id), () => unreachableOr(sharedConnectionNow(), 'fail_stop_helper_unreachable', t('fail_stop_helper_no_reply', { helper }), { helper }))) setConfirming(false)
          }}
        />
      )}
    </>
  )
}

export function ResumeButton({ session: s, action, prompt }: ActionProps & { prompt?: string }) {
  return (
    <button
      type="button"
      disabled={action.busy}
      onClick={() => void action.run(() => globalSessionsApi.resume(s.id, prompt?.trim() || undefined), t('helpers_resume_failed', { helper: s.label || s.id }))}
      className="a-btn"
    >
      {action.busy ? Spinner : <Play className="a-ic" aria-hidden="true" />}
      {t('helpers_resume_act')}
    </button>
  )
}

export function ArchiveButton({ session: s, action }: ActionProps) {
  const Icon = s.archived ? ArchiveRestore : Archive
  return (
    <button
      type="button"
      disabled={action.busy}
      onClick={() => void action.run(() => globalSessionsApi.archive(s.id, !s.archived), t('helpers_archive_one_failed', { helper: s.label || s.id }))}
      className={ACTION_BUTTON_CLASS}
    >
      {action.busy ? Spinner : <Icon className="a-ic" aria-hidden="true" />}
      {s.archived ? t('helpers_restore') : t('helpers_archive_act')}
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
    }, t('helpers_follow_failed'))

  if (threadId) {
    return (
      <Link
        to="/chat"
        onClick={() => useThreadStore.getState().setActiveThread(threadId)}
        className={ACTION_BUTTON_CLASS}
        title={t('helpers_follow_open')}
      >
        <Eye className="a-ic" aria-hidden="true" />
        {t('helpers_follow_open')}
      </Link>
    )
  }
  return (
    <button
      type="button"
      onClick={() => void follow()}
      disabled={action.busy}
      className={ACTION_BUTTON_CLASS}
      title={t('helpers_follow')}
    >
      {action.busy ? Spinner : <Eye className="a-ic" aria-hidden="true" />}
      {t('helpers_follow')}
    </button>
  )
}

export function ActionError({ error }: { error: string | null }) {
  return error ? <ErrorText>{error}</ErrorText> : null
}
