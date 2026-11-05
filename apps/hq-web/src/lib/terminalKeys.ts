import { INTERRUPT_KEY } from './sessionsApi'

export interface KeyLike {
  key: string
  code?: string
  ctrlKey?: boolean
  metaKey?: boolean
  altKey?: boolean
}

/** What a key pressed inside the helper's screen does. */
export type TerminalAction =
  | { do: 'leave' }
  | { do: 'send'; key: string }
  | { do: 'interrupt' }
  | { do: 'none' }

/** How long a first Ctrl C stays armed for the second press. */
export const INTERRUPT_ARM_MS = 4_000

const SENT: Record<string, string> = { Escape: 'esc', Enter: 'enter', ArrowUp: 'up', ArrowDown: 'down', y: 'y', n: 'n' }

/**
 * Ctrl ] is read by its physical key so it works on every layout, and always leaves. The rest go to the
 * helper only while "Typing to the helper" is on; a Ctrl C with text selected is a copy, not an interrupt.
 */
export function terminalAction(e: KeyLike, typing: boolean, hasSelection: boolean): TerminalAction {
  if (e.ctrlKey && (e.code === 'BracketRight' || e.key === ']')) return { do: 'leave' }
  if (!typing) return e.key === 'Escape' ? { do: 'leave' } : { do: 'none' }
  if (e.ctrlKey && e.key.toLowerCase() === 'c') return hasSelection ? { do: 'none' } : { do: 'interrupt' }
  if (e.ctrlKey || e.metaKey || e.altKey) return { do: 'none' }
  const key = SENT[e.key]
  return key ? { do: 'send', key } : { do: 'none' }
}

/** The second Ctrl C within the window sends; the first only arms. */
export function interruptStep(armedAt: number | null, now: number): { send: boolean; armedAt: number | null } {
  if (armedAt !== null && now - armedAt <= INTERRUPT_ARM_MS) return { send: true, armedAt: null }
  return { send: false, armedAt: now }
}

export const INTERRUPT = INTERRUPT_KEY
