import type { RefObject } from 'react'
import type { UseConnectionResult } from '~/lib/connection'

export interface StatusLineProps {
  /** The shell's one connection state, shared with the sheet so HQ is asked once. */
  connection: UseConnectionResult
  onClick?: () => void
  buttonRef?: RefObject<HTMLButtonElement | null>
}

export function StatusLine({ connection, onClick, buttonRef }: StatusLineProps) {
  return (
    <button
      ref={buttonRef}
      type="button"
      className="a-status"
      onClick={onClick}
    >
      <span className={`a-dot ${connection.dot}`} />
      <span>{connection.words}</span>
    </button>
  )
}
