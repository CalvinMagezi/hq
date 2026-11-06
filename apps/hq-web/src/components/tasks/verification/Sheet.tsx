import type { ReactNode } from 'react'
import { Sheet as SharedSheet } from '~/components/ui/Sheet'

interface Props {
  title: string
  onClose: () => void
  children: ReactNode
}

/** The checking sheets (sweep, audit) on the one shared sheet: centred from 768 px, a bottom sheet below. */
export function Sheet({ title, onClose, children }: Props) {
  return (
    <SharedSheet title={title} onClose={onClose} placement="center">
      {children}
    </SharedSheet>
  )
}

/** The design's buttons inside a sheet: the main action, the quiet one and the one that destroys. */
export const SHEET_PRIMARY = 'a-btn'
export const SHEET_QUIET = 'a-btn secondary'
export const SHEET_DANGER = 'a-btn danger'
