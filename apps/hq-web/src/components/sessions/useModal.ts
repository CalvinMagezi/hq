import { useEffect, useRef, useState } from 'react'

const FOCUSABLE = 'button, input, select, textarea, summary, [href], [tabindex]:not([tabindex="-1"])'

/**
 * A dialog mounted in its own portal: the rest of the page is inert while it is open,
 * Escape calls `onEscape`, Tab stays inside, and focus returns to the opener afterwards.
 */
export function useModal(onEscape: () => void) {
  const dialogRef = useRef<HTMLDivElement>(null)
  const [portal] = useState(() => document.createElement('div'))
  const escapeRef = useRef(onEscape)
  escapeRef.current = onEscape

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null
    document.body.appendChild(portal)
    const hidden = Array.from(document.body.children).filter((el) => el !== portal && !el.hasAttribute('inert'))
    hidden.forEach((el) => el.setAttribute('inert', ''))
    dialogRef.current?.focus()
    return () => {
      hidden.forEach((el) => el.removeAttribute('inert'))
      portal.remove()
      opener?.focus()
    }
  }, [portal])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && escapeRef.current()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const trapTab = (e: React.KeyboardEvent) => {
    if (e.key !== 'Tab') return
    const items = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? []).filter(
      (el) => !el.hasAttribute('disabled') && el.offsetParent !== null
    )
    if (items.length === 0) return
    const first = items[0]
    const last = items[items.length - 1]
    const active = document.activeElement
    if (e.shiftKey && (active === first || active === dialogRef.current)) {
      e.preventDefault()
      last.focus()
    } else if (!e.shiftKey && active === last) {
      e.preventDefault()
      first.focus()
    }
  }

  return { dialogRef, portal, trapTab }
}
