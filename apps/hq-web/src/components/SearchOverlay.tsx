import { Suspense, lazy, useCallback, useEffect, useRef, useState } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { useEdition } from '~/lib/edition'
import { forgetSearch, readBack } from '~/lib/searchBack'
import type { OpenDetail } from './SearchPanel'

// The panel is the whole search UI and its dependencies; it is only needed once the person opens
// Search, so it loads then, not with the page.
const SearchPanel = lazy(() => import('./SearchPanel').then((m) => ({ default: m.SearchPanel })))

/** The one Search: Ctrl or Cmd K, the top bar field and the Notes pill open it over whatever place you are in. */
export function SearchOverlay() {
  const navigate = useNavigate()
  const lite = useEdition() === 'lite'
  const [open, setOpen] = useState(false)
  const [start, setStart] = useState<OpenDetail>({})
  const opener = useRef<Element | null>(null)

  useEffect(() => {
    const show = (detail: OpenDetail) => {
      opener.current = document.activeElement
      setStart(detail)
      setOpen(true)
    }
    const onKey = (e: KeyboardEvent) => {
      // New chat has a key of its own, so the palette's action shows a chord that works.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && e.key.toLowerCase() === 'o') {
        e.preventDefault()
        if (!lite) void navigate({ to: '/chat', search: { compose: '1' } })
        return
      }
      // Every key is listed on the Chat page of Settings.
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key === '/') {
        e.preventDefault()
        if (!lite) void navigate({ to: '/settings/chat' })
        return
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        if (open) setOpen(false)
        else show({})
      }
    }
    const onOpen = (e: Event) => show((e as CustomEvent<OpenDetail | null>).detail ?? {})
    // The desktop app's menu says New chat with this event, since its key never reaches the page.
    const onNewChat = () => {
      if (!lite) void navigate({ to: '/chat', search: { compose: '1' } })
    }
    // Back from an opened result lands on the entry marked at that time: Search reopens once, as it was left.
    const onPop = () => {
      const back = readBack(window.location.search, window.history.state)
      if (!back) return
      forgetSearch()
      show({ q: back.q, scope: back.scope, ...(back.folder ? { folder: back.folder } : {}), scroll: back.scroll })
    }
    window.addEventListener('keydown', onKey)
    window.addEventListener('hq:open-search', onOpen)
    window.addEventListener('hq:new-chat', onNewChat)
    window.addEventListener('popstate', onPop)
    return () => {
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('hq:open-search', onOpen)
      window.removeEventListener('hq:new-chat', onNewChat)
      window.removeEventListener('popstate', onPop)
    }
  }, [open, navigate, lite])

  const close = useCallback(() => {
    setOpen(false)
    const back = opener.current
    if (back instanceof HTMLElement) requestAnimationFrame(() => back.focus())
  }, [])

  if (!open) return null
  return (
    <Suspense fallback={null}>
      <SearchPanel start={start} close={close} />
    </Suspense>
  )
}
