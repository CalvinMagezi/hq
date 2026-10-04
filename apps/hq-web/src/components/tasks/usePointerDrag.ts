import { useCallback, useEffect, useRef } from 'react'

// Pointer events instead of HTML5 drag-and-drop, which doesn't fire reliably
// on touch in mobile Safari. Mouse and pen drags start once the pointer moves
// past a small threshold; touch drags start after a long press so a normal
// swipe still scrolls the page.
const DRAG_THRESHOLD_PX = 4
const LONG_PRESS_MS = 350

export interface DragPoint {
  x: number
  y: number
  dx: number
  dy: number
}

interface PointerDragHandlers<T> {
  onStart?: (payload: T, point: DragPoint) => void
  onMove: (payload: T, point: DragPoint) => void
  onEnd: (payload: T, point: DragPoint) => void
  onCancel?: (payload: T) => void
  /** A press that never became a drag. */
  onClick?: (payload: T) => void
}

interface Session<T> {
  payload: T
  pointerId: number
  originX: number
  originY: number
  active: boolean
  isTouch: boolean
  timer: ReturnType<typeof setTimeout> | null
}

function preventScroll(e: TouchEvent) {
  e.preventDefault()
}

/** Returns a `pointerdown` handler; call it with the item being dragged. */
export function usePointerDrag<T>(handlers: PointerDragHandlers<T>) {
  const handlersRef = useRef(handlers)
  handlersRef.current = handlers
  const sessionRef = useRef<Session<T> | null>(null)
  const cleanupRef = useRef<(() => void) | null>(null)

  useEffect(() => () => cleanupRef.current?.(), [])

  return useCallback((e: React.PointerEvent, payload: T) => {
    if (e.button !== 0 || sessionRef.current) return
    e.stopPropagation()

    const session: Session<T> = {
      payload,
      pointerId: e.pointerId,
      originX: e.clientX,
      originY: e.clientY,
      active: false,
      isTouch: e.pointerType === 'touch',
      timer: null,
    }
    sessionRef.current = session

    const point = (ev: { clientX: number; clientY: number }): DragPoint => ({
      x: ev.clientX,
      y: ev.clientY,
      dx: ev.clientX - session.originX,
      dy: ev.clientY - session.originY,
    })

    const activate = (ev: { clientX: number; clientY: number }) => {
      session.active = true
      document.addEventListener('touchmove', preventScroll, { passive: false })
      handlersRef.current.onStart?.(session.payload, point(ev))
    }

    const finish = () => {
      if (session.timer) clearTimeout(session.timer)
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
      document.removeEventListener('touchmove', preventScroll)
      sessionRef.current = null
      cleanupRef.current = null
    }

    const onMove = (ev: PointerEvent) => {
      if (ev.pointerId !== session.pointerId) return
      const p = point(ev)
      const moved = Math.hypot(p.dx, p.dy) > DRAG_THRESHOLD_PX
      if (!session.active) {
        if (!moved) return
        // Moving before the long press fires means the user is scrolling.
        if (session.isTouch) return finish()
        activate(ev)
      }
      handlersRef.current.onMove(session.payload, p)
    }

    const onUp = (ev: PointerEvent) => {
      if (ev.pointerId !== session.pointerId) return
      const wasActive = session.active
      finish()
      if (wasActive) handlersRef.current.onEnd(session.payload, point(ev))
      else handlersRef.current.onClick?.(session.payload)
    }

    const onCancel = (ev: PointerEvent) => {
      if (ev.pointerId !== session.pointerId) return
      const wasActive = session.active
      finish()
      if (wasActive) handlersRef.current.onCancel?.(session.payload)
    }

    if (session.isTouch) {
      const start = { clientX: e.clientX, clientY: e.clientY }
      session.timer = setTimeout(() => activate(start), LONG_PRESS_MS)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
    cleanupRef.current = finish
  }, [])
}
