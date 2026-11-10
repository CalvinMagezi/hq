// Opt-in trace of the thread's scrolling for chasing WebKit-only jumps: open the app with `?scrolldebug`.
const FLAG = 'scrolldebug'
const SHOWN_LINES = 30

interface DebugWindow {
  __scrollLog?: string[]
}

let overlay: HTMLPreElement | null = null

const enabled = () => typeof window !== 'undefined' && window.location.search.includes(FLAG)

function ensureOverlay(): HTMLPreElement {
  if (overlay?.isConnected) return overlay
  overlay = document.createElement('pre')
  overlay.className = 'hq-scrolldebug'
  document.body.appendChild(overlay)
  return overlay
}

/** Adds one line to the trace; does nothing unless the flag is in the address. */
export function scrollNote(message: string) {
  if (!enabled()) return
  const log = ((window as unknown as DebugWindow).__scrollLog ??= [])
  log.push(`${Math.round(performance.now())} ${message}`)
  ensureOverlay().textContent = log.slice(-SHOWN_LINES).join('\n')
}

/** Traces every scroll and every size change of the thread. Returns the cleanup. */
export function traceScroller(el: HTMLElement, content: HTMLElement): () => void {
  if (!enabled()) return () => {}
  const state = () => `top=${Math.round(el.scrollTop)} sh=${el.scrollHeight} ch=${el.clientHeight}`
  const onScroll = () => scrollNote(`scroll ${state()}`)
  const observer = new ResizeObserver(() => scrollNote(`resize ${state()}`))
  el.addEventListener('scroll', onScroll, { passive: true })
  observer.observe(content)
  observer.observe(el)
  return () => {
    el.removeEventListener('scroll', onScroll)
    observer.disconnect()
  }
}
