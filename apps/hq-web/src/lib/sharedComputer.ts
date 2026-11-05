// "This is a shared computer" (rules-web.md 3, shared parts rule 10): the sign-in lasts for this tab
// only, nothing from HQ is saved, nothing waits to send, push is never offered, and the tab signs
// out after 30 minutes with no input.

const FLAG_KEY = 'hq-shared'
export const SHARED_IDLE_MS = 30 * 60 * 1000
const INPUT_EVENTS = ['pointerdown', 'keydown', 'wheel', 'touchstart'] as const

function session(): Storage | null {
  try {
    return typeof sessionStorage === 'undefined' ? null : sessionStorage
  } catch {
    return null
  }
}

export function isSharedComputer(): boolean {
  return session()?.getItem(FLAG_KEY) === '1'
}

export function setSharedComputer(on: boolean) {
  const s = session()
  if (!s) return
  if (on) s.setItem(FLAG_KEY, '1')
  else s.removeItem(FLAG_KEY)
}

/** Where the sign-in is kept: this tab only on a shared computer, else the browser. */
export function tokenStorage(): Storage | null {
  if (isSharedComputer()) return session()
  try {
    return typeof localStorage === 'undefined' ? null : localStorage
  } catch {
    return null
  }
}

/** Calls `onIdle` once 30 minutes pass with no input. Returns the stop. */
export function watchIdle(onIdle: () => void, target: Pick<Window, 'addEventListener' | 'removeEventListener'> = window, idleMs = SHARED_IDLE_MS): () => void {
  let timer = setTimeout(onIdle, idleMs)
  const reset = () => {
    clearTimeout(timer)
    timer = setTimeout(onIdle, idleMs)
  }
  for (const e of INPUT_EVENTS) target.addEventListener(e, reset, { passive: true })
  return () => {
    clearTimeout(timer)
    for (const e of INPUT_EVENTS) target.removeEventListener(e, reset)
  }
}
