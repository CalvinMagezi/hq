import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react'

import { scrollNote, traceScroller } from './scrollDebug'

// Within this many pixels of the end still counts as "at the bottom" (momentum scrolling overshoots).
const AT_BOTTOM_SLACK_PX = 80
// A wheel, finger or key moved the list this recently, so a move up is the reader's own.
const INTENT_MS = 400
const SCROLL_KEYS = new Set(['ArrowUp', 'PageUp', 'Home'])

/**
 * Whether the view stops following. Only the reader moving up lets go: WebKit reports scrolls late,
 * so a position from before content grew also looks like a move up.
 */
export function letsGo(near: boolean, top: number, lastTop: number, readerMoved: boolean): boolean {
  return !near && top < lastTop && readerMoved
}

interface StickToBottom {
  /** The user is at (or near) the newest message, or the view is still following it. */
  atBottom: boolean
  /** New chat activity arrived while the user was scrolled up. */
  hasUnseen: boolean
  /** Scroll to the newest message and follow it again. */
  scrollToBottom: (smooth?: boolean) => void
}

/**
 * Follows new content only while the user is at the bottom, so reading an
 * earlier message is never yanked away. A ResizeObserver on the content catches
 * everything that grows it: streamed text, tool steps, opened panels, images.
 * `resetKey` (the open chat) jumps straight to the end when it changes, and
 * `activityKey` (messages and streamed text) marks "new messages" while scrolled
 * up, so opening a panel yourself never does.
 */
export function useStickToBottom(
  scrollRef: RefObject<HTMLElement | null>,
  contentRef: RefObject<HTMLElement | null>,
  resetKey: unknown,
  activityKey: unknown,
): StickToBottom {
  const stickRef = useRef(true)
  const [atBottom, setAtBottom] = useState(true)
  const [hasUnseen, setHasUnseen] = useState(false)

  const scrollToBottom = useCallback(
    (smooth = false) => {
      const el = scrollRef.current
      if (!el) return
      stickRef.current = true
      setAtBottom(true)
      setHasUnseen(false)
      scrollNote(`scrollToBottom smooth=${smooth}`)
      el.scrollTo({ top: el.scrollHeight, behavior: smooth ? 'smooth' : 'auto' })
    },
    [scrollRef],
  )

  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    let lastTop = el.scrollTop
    let movedAt = 0
    let dragging = false
    const moved = () => {
      movedAt = performance.now()
    }
    const onWheel = (e: WheelEvent) => {
      if (e.deltaY < 0) moved()
    }
    const onKey = (e: KeyboardEvent) => {
      if (SCROLL_KEYS.has(e.key)) moved()
    }
    // A press on the scroller itself, not on anything inside it, is its scrollbar.
    const onPress = (e: PointerEvent) => {
      if (e.target === el) dragging = true
    }
    const onRelease = () => {
      dragging = false
    }
    const onScroll = () => {
      const near = el.scrollHeight - el.scrollTop - el.clientHeight <= AT_BOTTOM_SLACK_PX
      if (near) stickRef.current = true
      else if (el.scrollTop < lastTop) {
        const readerMoved = dragging || performance.now() - movedAt < INTENT_MS
        scrollNote(`up by ${Math.round(lastTop - el.scrollTop)} reader=${readerMoved} follow=${stickRef.current}`)
        if (letsGo(near, el.scrollTop, lastTop, readerMoved)) stickRef.current = false
        // A jump up nobody asked for, while following, would strand the view until the next resize.
        else if (stickRef.current) el.scrollTop = el.scrollHeight
      }
      lastTop = el.scrollTop
      setAtBottom(near || stickRef.current)
      if (near) setHasUnseen(false)
    }
    el.addEventListener('scroll', onScroll, { passive: true })
    el.addEventListener('wheel', onWheel, { passive: true })
    el.addEventListener('touchmove', moved, { passive: true })
    el.addEventListener('keydown', onKey)
    el.addEventListener('pointerdown', onPress)
    window.addEventListener('pointerup', onRelease)
    return () => {
      el.removeEventListener('scroll', onScroll)
      el.removeEventListener('wheel', onWheel)
      el.removeEventListener('touchmove', moved)
      el.removeEventListener('keydown', onKey)
      el.removeEventListener('pointerdown', onPress)
      window.removeEventListener('pointerup', onRelease)
    }
  }, [scrollRef])

  useEffect(() => {
    const el = scrollRef.current
    const content = contentRef.current
    if (!el || !content) return
    const observer = new ResizeObserver(() => {
      if (stickRef.current) el.scrollTop = el.scrollHeight
    })
    observer.observe(content)
    const stopTrace = traceScroller(el, content)
    // The keyboard opening on a phone shrinks the scroller itself.
    observer.observe(el)
    return () => {
      observer.disconnect()
      stopTrace()
    }
  }, [scrollRef, contentRef])

  useLayoutEffect(() => {
    scrollToBottom(false)
  }, [resetKey, scrollToBottom])

  useEffect(() => {
    if (!stickRef.current) setHasUnseen(true)
  }, [activityKey])

  return { atBottom, hasUnseen, scrollToBottom }
}
