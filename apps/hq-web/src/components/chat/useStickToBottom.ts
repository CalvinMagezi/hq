import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react'

// Within this many pixels of the end still counts as "at the bottom" (momentum scrolling overshoots).
const AT_BOTTOM_SLACK_PX = 80

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
      el.scrollTo({ top: el.scrollHeight, behavior: smooth ? 'smooth' : 'auto' })
    },
    [scrollRef],
  )

  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    let lastTop = el.scrollTop
    const onScroll = () => {
      const near = el.scrollHeight - el.scrollTop - el.clientHeight <= AT_BOTTOM_SLACK_PX
      // Only moving up lets go. Scroll anchoring moves down when content above
      // grows (markdown rendering in), and that must not stop the follow.
      if (near) stickRef.current = true
      else if (el.scrollTop < lastTop) stickRef.current = false
      lastTop = el.scrollTop
      setAtBottom(near || stickRef.current)
      if (near) setHasUnseen(false)
    }
    el.addEventListener('scroll', onScroll, { passive: true })
    return () => el.removeEventListener('scroll', onScroll)
  }, [scrollRef])

  useEffect(() => {
    const el = scrollRef.current
    const content = contentRef.current
    if (!el || !content) return
    const observer = new ResizeObserver(() => {
      if (stickRef.current) el.scrollTop = el.scrollHeight
    })
    observer.observe(content)
    // The keyboard opening on a phone shrinks the scroller itself.
    observer.observe(el)
    return () => observer.disconnect()
  }, [scrollRef, contentRef])

  useLayoutEffect(() => {
    scrollToBottom(false)
  }, [resetKey, scrollToBottom])

  useEffect(() => {
    if (!stickRef.current) setHasUnseen(true)
  }, [activityKey])

  return { atBottom, hasUnseen, scrollToBottom }
}
