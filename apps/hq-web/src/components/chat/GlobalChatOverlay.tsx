import { useEffect } from 'react'
import { useHQStore } from '~/store/hqStore'
import { ChatView } from './ChatView'

/** The chat, full screen over any page (Cmd+K). Same ChatView as the /chat route. */
export function GlobalChatOverlay() {
  const open = useHQStore((s) => s.globalChatOpen)
  const setOpen = useHQStore((s) => s.setGlobalChatOpen)
  const clearChatUnread = useHQStore((s) => s.clearChatUnread)

  useEffect(() => {
    if (!open) return
    clearChatUnread()
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, clearChatUnread, setOpen])

  // Stays mounted while hidden so a half-written message survives closing.
  return (
    <div
      className={`fixed inset-0 z-[150] pad-safe-top pb-[var(--safe-bottom)] bg-neutral-950/95 backdrop-blur-2xl ${open ? '' : 'hidden'}`}
      role="dialog"
      aria-label="Chat"
      aria-hidden={!open}
    >
      <ChatView active={open} onClose={() => setOpen(false)} />
    </div>
  )
}
