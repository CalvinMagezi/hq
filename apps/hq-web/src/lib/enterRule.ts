const KEY_PROCESS = 229
const STORAGE_KEY = 'hq.chat.enterSends'

export interface EnterInput {
  key: string
  shiftKey: boolean
  isComposing: boolean
  keyCode: number
  /** A touch-first screen: there is no Shift key for a newline. */
  coarse: boolean
  paletteActive: boolean
  /** The owner's choice, or null for the screen's default. */
  enterSends: boolean | null
}

/** Whether this Enter sends the message. Composing (an IME's Enter, keyCode 229) and Shift Enter never do. */
export function enterSendsMessage(e: EnterInput): boolean {
  if (e.key !== 'Enter' || e.shiftKey || e.isComposing || e.keyCode === KEY_PROCESS) return false
  if (e.paletteActive) return false
  return e.enterSends ?? !e.coarse
}

/** The "Enter sends" choice kept in this browser, or null when it was never set. */
export function readEnterSends(): boolean | null {
  try {
    const v = window.localStorage.getItem(STORAGE_KEY)
    return v === null ? null : v === '1'
  } catch {
    return null
  }
}

export function writeEnterSends(on: boolean) {
  try {
    window.localStorage.setItem(STORAGE_KEY, on ? '1' : '0')
  } catch {
    // A browser that refuses storage keeps the default.
  }
}
