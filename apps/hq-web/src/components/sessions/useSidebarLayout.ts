import { useCallback, useEffect, useRef, useState } from 'react'
import {
  DEFAULT_SIDEBAR,
  SIDEBAR_DEFAULT,
  SIDEBAR_KEY_STEP,
  clampSidebarWidth,
  parseSidebarLayout,
  type SidebarLayout,
} from '~/lib/sidebarLayout'

const STORAGE_KEY = 'hq.workbench.sidebar'

/**
 * Width and collapsed state of the Workbench sidebar, remembered on this device.
 * Read in an effect, never during render, so the prerender and first client paint agree.
 */
export function useSidebarLayout() {
  const [layout, setLayout] = useState<SidebarLayout>(DEFAULT_SIDEBAR)
  const [dragging, setDragging] = useState(false)
  const layoutRef = useRef(layout)
  layoutRef.current = layout

  useEffect(() => {
    try {
      setLayout(parseSidebarLayout(window.localStorage.getItem(STORAGE_KEY)))
    } catch {
      // Storage can be blocked; the default layout still works.
    }
  }, [])

  const commit = useCallback((next: SidebarLayout) => {
    setLayout(next)
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
    } catch {
      // Not remembering is fine.
    }
  }, [])

  const toggle = useCallback(() => commit({ ...layoutRef.current, collapsed: !layoutRef.current.collapsed }), [commit])
  const reset = useCallback(() => commit({ width: SIDEBAR_DEFAULT, collapsed: false }), [commit])

  const startDrag = useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      event.preventDefault()
      const startX = event.clientX
      const startWidth = layoutRef.current.width
      setDragging(true)
      const move = (e: PointerEvent) => setLayout({ collapsed: false, width: clampSidebarWidth(startWidth + e.clientX - startX) })
      const end = () => {
        window.removeEventListener('pointermove', move)
        window.removeEventListener('pointerup', end)
        window.removeEventListener('pointercancel', end)
        setDragging(false)
        commit(layoutRef.current)
      }
      window.addEventListener('pointermove', move)
      window.addEventListener('pointerup', end)
      window.addEventListener('pointercancel', end)
    },
    [commit],
  )

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLElement>) => {
      const delta = event.key === 'ArrowLeft' ? -SIDEBAR_KEY_STEP : event.key === 'ArrowRight' ? SIDEBAR_KEY_STEP : 0
      if (!delta) return
      event.preventDefault()
      commit({ collapsed: false, width: clampSidebarWidth(layoutRef.current.width + delta) })
    },
    [commit],
  )

  return { ...layout, dragging, toggle, reset, startDrag, onKeyDown }
}
