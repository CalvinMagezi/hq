import { useEffect, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { X } from 'lucide-react'

const SOLID = { background: 'var(--bg-solid-elevated)' }

interface Props {
  title: string
  onClose: () => void
  children: ReactNode
}

/**
 * A bottom sheet on a phone and a centred panel on a desktop. The body scrolls and the title bar
 * stays, so the close button is always under the thumb's reach. Escape closes it.
 */
export function Sheet({ title, onClose, children }: Props) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  if (typeof document === 'undefined') return null
  return createPortal(
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center hq-scrim" role="dialog" aria-modal="true" aria-label={title}>
      <button type="button" aria-label="Close" className="absolute inset-0 cursor-default" onClick={onClose} />
      {/* A solid fill, inline because the unlayered `.hq-modal` background would beat a utility: a
          sheet is read, and the page behind it must not show through the text. */}
      <div
        style={SOLID}
        className="relative w-full sm:max-w-2xl max-h-[92dvh] sm:max-h-[85vh] flex flex-col hq-modal rounded-t-3xl sm:rounded-3xl shadow-2xl overflow-hidden pb-[var(--safe-bottom)]"
      >
        <div className="flex items-center justify-between gap-3 px-5 py-3 border-b border-white/10 shrink-0">
          <h2 className="text-base font-semibold text-white truncate" style={{ fontFamily: 'var(--font-heading)' }}>
            {title}
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="p-2.5 -mr-2 rounded-xl text-neutral-400 hover:text-white hover:bg-white/10"
          >
            <X className="w-5 h-5" />
          </button>
        </div>
        <div className="overflow-y-auto px-5 py-4 space-y-4">{children}</div>
      </div>
    </div>,
    document.body
  )
}

/** Large, thumb-sized buttons used inside sheets. */
export const SHEET_BUTTON = 'min-h-11 px-4 rounded-xl text-sm font-semibold flex items-center justify-center gap-2 disabled:opacity-40 disabled:pointer-events-none'
export const SHEET_PRIMARY = `${SHEET_BUTTON} bg-emerald-500 text-neutral-950 hover:bg-emerald-400`
export const SHEET_QUIET = `${SHEET_BUTTON} hq-field text-neutral-200 hover:text-white`
export const SHEET_DANGER = `${SHEET_BUTTON} bg-rose-500/90 text-white hover:bg-rose-400`
