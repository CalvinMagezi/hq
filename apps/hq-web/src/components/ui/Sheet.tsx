import { useEffect, useId, useRef, type ReactNode } from 'react'
import { X } from 'lucide-react'
import { t } from '~/strings'

/** Where the sheet sits from 768 px: beside the page (`side`, 440 wide) or in the middle (`center`). Below 768 it always rises from the bottom. */
export type SheetPlacement = 'side' | 'center'

export interface SheetProps {
  title: string
  onClose: () => void
  children: ReactNode
  placement?: SheetPlacement
  /** Buttons for `.a-sheet-foot`; the safe one first, the action last. */
  footer?: ReactNode
  /** A sheet that is working (saving, sending) cannot be closed until it is done. */
  busy?: boolean
}

/** Classes for the design's `.a-sheet`: a bottom sheet with its handle below 768 px, a side or centred sheet above. */
export function sheetClass(placement: SheetPlacement = 'center'): string {
  return `a-sheet ${placement} hq-sheet`
}

/** The one sheet (AlignedFoundations 2): a native modal <dialog>, so focus stays inside, Esc and the scrim close it, and focus returns to what opened it. */
export function Sheet({ title, onClose, children, placement = 'center', footer, busy }: SheetProps) {
  const ref = useRef<HTMLDialogElement>(null)
  const titleId = useId()
  const closeRef = useRef(onClose)
  const busyRef = useRef(!!busy)
  closeRef.current = onClose
  busyRef.current = !!busy

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const trigger = document.activeElement as HTMLElement | null
    if (!el.open) el.showModal()
    return () => {
      if (el.open) el.close()
      trigger?.focus?.()
    }
  }, [])

  return (
    <dialog
      ref={ref}
      className={sheetClass(placement)}
      aria-labelledby={titleId}
      onCancel={(e) => {
        e.preventDefault()
        if (!busyRef.current) closeRef.current()
      }}
      onClick={(e) => {
        // A click on the scrim lands on the dialog itself, outside its box.
        if (e.target !== e.currentTarget || busyRef.current) return
        const r = e.currentTarget.getBoundingClientRect()
        const inside = Math.min(e.clientX - r.left, r.right - e.clientX, e.clientY - r.top, r.bottom - e.clientY) >= 0
        if (!inside) closeRef.current()
      }}
    >
      <span className="a-handle" aria-hidden="true" />
      <div className="a-sheet-head">
        <h2 className="a-h2" id={titleId}>
          {title}
        </h2>
        <button type="button" className="a-ib" aria-label={t('close')} disabled={busy} onClick={onClose}>
          <X className="a-ic" aria-hidden="true" />
        </button>
      </div>
      <div className="a-sheet-body">{children}</div>
      {footer && <div className="a-sheet-foot">{footer}</div>}
    </dialog>
  )
}
