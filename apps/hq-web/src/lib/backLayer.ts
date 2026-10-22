import { useEffect, useRef } from 'react'
import { readEmbedFlag } from '~/lib/embedHost'

// A drawer or a form that sits over a page is one more step for Back inside the Android app's own screen:
// opening it adds a history entry, so Back closes it before it leaves the page.
const LAYER_KEY = '__hqLayer'
let nextLayer = 0

/** True when the history entry on top is the one this layer pushed. */
export function layerIsOnTop(state: unknown, marker: string): boolean {
  return typeof state === 'object' && state !== null && (state as Record<string, unknown>)[LAYER_KEY] === marker
}

/** Back closes the layer while [open]. Only inside the Android host, where the page has no Back button of its own. */
export function useBackClosesLayer(open: boolean, close: () => void): void {
  const closeRef = useRef(close)
  closeRef.current = close
  useEffect(() => {
    if (!open || !readEmbedFlag()) return
    const marker = `layer-${nextLayer++}`
    // The router keeps its own position in the state; it is copied so the router sees the same place.
    window.history.pushState({ ...(window.history.state ?? {}), [LAYER_KEY]: marker }, '')
    let closedByBack = false
    const onPop = () => {
      if (layerIsOnTop(window.history.state, marker)) return
      closedByBack = true
      closeRef.current()
    }
    window.addEventListener('popstate', onPop)
    return () => {
      window.removeEventListener('popstate', onPop)
      // Closed by the page itself: the entry it added is taken off again, unless Back already did.
      if (!closedByBack && layerIsOnTop(window.history.state, marker)) window.history.back()
    }
  }, [open])
}
