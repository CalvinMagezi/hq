import { useCallback, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { usePersistedState } from '~/lib/usePersistedState'

const SIDEBAR_DEFAULT_WIDTH = 288
const SIDEBAR_MIN_WIDTH = 220
const SIDEBAR_MAX_WIDTH = 560

const isWidth = (v: unknown) => typeof v === 'number' && v >= SIDEBAR_MIN_WIDTH && v <= SIDEBAR_MAX_WIDTH

/** A drag-resizable sidebar width that survives reloads; double-click resets it. */
export function useSidebarResize(storageKey: string) {
  const [width, setWidth] = usePersistedState(storageKey, SIDEBAR_DEFAULT_WIDTH, isWidth)
  const [isDragging, setIsDragging] = useState(false)
  const isDraggingRef = useRef(false)
  const startXRef = useRef(0)
  const startWidthRef = useRef(SIDEBAR_DEFAULT_WIDTH)

  const onResizerMouseDown = useCallback(
    (e: ReactMouseEvent) => {
      e.preventDefault()
      isDraggingRef.current = true
      startXRef.current = e.clientX
      startWidthRef.current = width
      setIsDragging(true)
      document.body.style.cursor = 'col-resize'
      document.body.style.userSelect = 'none'

      const handleMouseMove = (ev: MouseEvent) => {
        if (!isDraggingRef.current) return
        const delta = ev.clientX - startXRef.current
        setWidth(Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, startWidthRef.current + delta)))
      }

      const handleMouseUp = () => {
        if (!isDraggingRef.current) return
        isDraggingRef.current = false
        setIsDragging(false)
        document.body.style.cursor = ''
        document.body.style.userSelect = ''
        window.removeEventListener('mousemove', handleMouseMove)
        window.removeEventListener('mouseup', handleMouseUp)
      }

      window.addEventListener('mousemove', handleMouseMove)
      window.addEventListener('mouseup', handleMouseUp)
    },
    [width, setWidth],
  )

  const onResizerDoubleClick = useCallback(() => setWidth(SIDEBAR_DEFAULT_WIDTH), [setWidth])

  return { width, isDragging, onResizerMouseDown, onResizerDoubleClick }
}
