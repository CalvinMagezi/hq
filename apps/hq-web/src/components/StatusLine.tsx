import type { RefObject } from 'react'
import type { ConnectionState, UseConnectionResult } from '~/lib/connection'
import { t } from '~/strings'

export interface StatusLineProps {
  /** The shell's one connection state, shared with the sheet so HQ is asked once. */
  connection: UseConnectionResult
  /** At most one fact of the place, shown after the state. */
  fact?: string | null
  onClick?: () => void
  buttonRef?: RefObject<HTMLButtonElement | null>
}

export function StatusLine({ connection, fact, onClick, buttonRef }: StatusLineProps) {
  const words = fact ? t('status_with_fact', { state: connection.words, fact }) : connection.words
  return (
    <button
      ref={buttonRef}
      type="button"
      className="a-status"
      aria-label={t('status_aria', { state: words })}
      onClick={onClick}
    >
      <span className={`a-dot ${connection.dot}`} aria-hidden="true" />
      <span>{words}</span>
    </button>
  )
}

/** The place's name for the header, from the path. */
export function placeTitle(pathname: string): string {
  const first = pathname.split('/')[1] ?? ''
  switch (first) {
    case '':
    case 'chat':
      return t('place_chat')
    case 'tasks':
      return t('place_tasks')
    case 'notes':
      return t('place_notes')
    case 'helpers':
      return t('place_helpers')
    case 'inbox':
      return t('place_inbox')
    case 'settings':
      return t('place_settings')
    default:
      return t('place_other')
  }
}

/** The place's one fact for the status line: tasks in progress on Tasks, helpers working on the Workbench. */
export function placeFact(pathname: string, counts: { activeTasks: number; workingHelpers: number }): string | null {
  const first = pathname.split('/')[1] ?? ''
  if (first === 'tasks' && counts.activeTasks > 0) return t('fact_active', { n: counts.activeTasks })
  if (first === 'helpers' && counts.workingHelpers > 0) return t('fact_working', { n: counts.workingHelpers })
  return null
}

export interface HeaderTitleProps extends StatusLineProps {
  title: string
}

/** The place's title with the status line under it. The line wraps, never cuts. */
export function HeaderTitle({ title, ...line }: HeaderTitleProps) {
  return (
    <div className="a-tt">
      <span className="a-title">{title}</span>
      <StatusLine {...line} />
    </div>
  )
}

/** A balance or limit chip shows only while Connected; a limit's words are amber, never red. */
export function BalanceChip({ state, label, title, limit }: { state: ConnectionState; label: string | null; title?: string; limit?: boolean }) {
  if (state !== 'connected' || !label) return null
  return (
    <span className={`a-bal ${limit ? 'warn' : ''}`} title={title}>
      <span>{label}</span>
    </span>
  )
}
