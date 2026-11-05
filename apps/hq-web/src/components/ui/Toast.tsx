import { useEffect, useState } from 'react'
import { X } from 'lucide-react'
import { t } from '~/strings'
import { useHQStore } from '~/store/hqStore'
import {
  NOTICE_MS,
  TOAST_GAP,
  UNDO_WINDOW_MS,
  pressUndo,
  toastBottom,
  undoTooLate,
  useToasts,
  type Box,
  type Toast,
} from '~/lib/toast'

const WIDE_FROM = 768
const SNACK_HEIGHT = 48
const RESTING_PHONE = 72
const RESTING_WIDE = 24
const FIELD = 'input, textarea, select, [contenteditable=""], [contenteditable="true"]'

/** The placement rule: full width above the bottom bar on a phone, bottom right from 768. */
export function toastPlacement(viewportWidth: number): { wide: boolean; resting: number } {
  const wide = viewportWidth >= WIDE_FROM
  return { wide, resting: wide ? RESTING_WIDE : RESTING_PHONE }
}

/** One toast. `bottom` is the offset the host computed so it never covers the focused field. */
export function ToastView({ toast, wide, bottom, onUndo, onClose }: { toast: Toast; wide: boolean; bottom: number; onUndo: () => void; onClose: () => void }) {
  const style = wide
    ? { position: 'fixed' as const, right: 24, bottom, left: 'auto', width: 'min(480px, 100% - 48px)' }
    : { position: 'fixed' as const, left: 8, right: 8, bottom }
  return (
    <div className="a-snack" style={style} role={toast.kind === 'alert' ? 'alert' : undefined}>
      <span className="whitespace-pre-line">{toast.text}</span>
      {toast.undo && (
        <button type="button" className="a-snack-act" aria-label={t('undo_named', { line: toast.text })} onClick={onUndo}>
          {t('undo')}
        </button>
      )}
      {toast.action && (
        <button type="button" className="a-snack-act" onClick={toast.action.run}>
          {toast.action.label}
        </button>
      )}
      <button type="button" className="a-ib" aria-label={t('close')} onClick={onClose}>
        <X className="w-5 h-5" aria-hidden="true" />
      </button>
    </div>
  )
}

function focusedFieldBox(): Box | null {
  const el = document.activeElement
  if (!el || !el.matches(FIELD)) return null
  const r = el.getBoundingClientRect()
  return { top: r.top, bottom: r.bottom, left: r.left, right: r.right }
}

/** The one toast host: the system notice, Undo toasts and failures, in a live region. */
export function ToastHost() {
  const toasts = useToasts((s) => s.toasts)
  const dismiss = useToasts((s) => s.dismiss)
  const replace = useToasts((s) => s.replace)
  const push = useToasts((s) => s.push)
  const notice = useHQStore((s) => s.systemNotice)
  const setNotice = useHQStore((s) => s.setSystemNotice)
  const [viewport, setViewport] = useState(() => ({ w: typeof window === 'undefined' ? 390 : window.innerWidth, h: typeof window === 'undefined' ? 800 : window.innerHeight }))
  const [field, setField] = useState<Box | null>(null)

  // The store's one-line notice shows here, so there is one place a toast appears.
  useEffect(() => {
    if (!notice) return
    push({ text: notice, kind: 'status' })
    setNotice(null)
  }, [notice, push, setNotice])

  useEffect(() => {
    const resize = () => setViewport({ w: window.innerWidth, h: window.innerHeight })
    const focus = () => setField(focusedFieldBox())
    window.addEventListener('resize', resize)
    document.addEventListener('focusin', focus)
    document.addEventListener('focusout', focus)
    return () => {
      window.removeEventListener('resize', resize)
      document.removeEventListener('focusin', focus)
      document.removeEventListener('focusout', focus)
    }
  }, [])

  // Plain notices go by themselves; Undo and failures stay until closed.
  useEffect(() => {
    const timers = toasts.map((x) => {
      if (x.kind === 'status' && !x.undo) return window.setTimeout(() => dismiss(x.id), NOTICE_MS)
      if (x.undo) {
        const left = Math.max(0, x.at + UNDO_WINDOW_MS - Date.now())
        return window.setTimeout(() => replace(x.id, { text: undoTooLate(), kind: 'alert', undo: undefined, late: true }), left)
      }
      return 0
    })
    return () => timers.forEach((id) => id && window.clearTimeout(id))
  }, [toasts, dismiss, replace])

  const { wide, resting } = toastPlacement(viewport.w)
  const shown = toasts.slice(-3)
  return (
    <div role="status" aria-live="polite">
      {shown.map((x, i) => {
        const above = (shown.length - 1 - i) * (SNACK_HEIGHT + TOAST_GAP)
        const bottom = toastBottom(resting, SNACK_HEIGHT, viewport.h, field) + above
        return <ToastView key={x.id} toast={x} wide={wide} bottom={bottom} onUndo={() => void pressUndo(x.id)} onClose={() => dismiss(x.id)} />
      })}
    </div>
  )
}
