// A timer that only runs while the tab is visible (data budget, rule 6): hidden stops it, shown
// runs `fn` once at once and starts it again.

interface DocLike {
  visibilityState: string
  addEventListener: (type: 'visibilitychange', cb: () => void) => void
  removeEventListener: (type: 'visibilitychange', cb: () => void) => void
}

export function visibleTimer(fn: () => void, everyMs: number, doc: DocLike = document): () => void {
  let timer: ReturnType<typeof setInterval> | undefined
  const stop = () => {
    clearInterval(timer)
    timer = undefined
  }
  const start = () => {
    if (timer === undefined) timer = setInterval(fn, everyMs)
  }
  const onChange = () => {
    if (doc.visibilityState === 'visible') {
      fn()
      start()
    } else stop()
  }
  doc.addEventListener('visibilitychange', onChange)
  if (doc.visibilityState === 'visible') start()
  return () => {
    stop()
    doc.removeEventListener('visibilitychange', onChange)
  }
}
