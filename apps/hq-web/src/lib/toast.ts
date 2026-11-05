import { create } from 'zustand'
import { t } from '~/strings'
import { HqHttpError } from './hqAuth'
import { count, type NoticedName } from './noticed'

/** HQ accepts a restore for 10 minutes after the change (rules-web 4). */
export const UNDO_WINDOW_MS = 600_000
/** A plain notice goes away by itself; one with Undo, or a failure, stays until closed. */
export const NOTICE_MS = 6_000
const HTTP_CONFLICT = 409
const NOTICED_VERSION = 'web'

export type ToastKind = 'status' | 'alert'

export interface Toast {
  id: number
  text: string
  kind: ToastKind
  /** Calls the restore route on HQ. Present only on a toast that offers Undo. */
  undo?: () => Promise<unknown>
  /** What the toast says once the restore went through. */
  restored?: string
  /** A plain button on the toast, such as Reload. */
  action?: { label: string; run: () => void }
  /** The counter an Undo press adds to in "What HQ noticed". */
  counter?: NoticedName
  /** When the change was made; the window runs from here. */
  at: number
  /** The Undo button is greyed out once the window is over. */
  late: boolean
}

interface ToastStore {
  toasts: Toast[]
  push: (toast: Omit<Toast, 'id' | 'at' | 'late'> & { at?: number }) => number
  replace: (id: number, next: Partial<Toast>) => void
  dismiss: (id: number) => void
}

let nextId = 1

export const useToasts = create<ToastStore>((set) => ({
  toasts: [],
  push: (toast) => {
    const id = nextId++
    set((s) => ({ toasts: [...s.toasts, { at: Date.now(), ...toast, id, late: false }] }))
    return id
  },
  replace: (id, next) => set((s) => ({ toasts: s.toasts.map((x) => (x.id === id ? { ...x, ...next } : x)) })),
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) })),
}))

export function undoWindowOpen(at: number, now = Date.now()): boolean {
  return now - at < UNDO_WINDOW_MS
}

/** A change HQ can put back: done at once, with an Undo that calls the restore route. */
export function toastUndo(text: string, undo: () => Promise<unknown>, restored: string, counter?: NoticedName): number {
  return useToasts.getState().push({ text, kind: 'status', undo, restored, counter })
}

/** A line with no Undo. */
export function toastNotice(text: string): number {
  return useToasts.getState().push({ text, kind: 'status' })
}

/** A failure: read out loud and kept until closed. */
export function toastAlert(text: string): number {
  return useToasts.getState().push({ text, kind: 'alert' })
}

/** The line a restore that HQ refused or that came too late shows. */
export function undoTooLate(): string {
  return t('fail_undo_too_late')
}

/** Presses Undo: too late and a 409 both say so; anything else keeps its own words. */
export async function pressUndo(id: number, now = Date.now()): Promise<'restored' | 'too_late' | 'failed' | 'none'> {
  const { toasts, replace } = useToasts.getState()
  const toast = toasts.find((x) => x.id === id)
  if (!toast?.undo) return 'none'
  if (!undoWindowOpen(toast.at, now)) {
    replace(id, { text: undoTooLate(), kind: 'alert', undo: undefined, late: true })
    return 'too_late'
  }
  try {
    await toast.undo()
  } catch (e) {
    if (e instanceof HqHttpError && e.status === HTTP_CONFLICT) {
      replace(id, { text: undoTooLate(), kind: 'alert', undo: undefined, late: true })
      return 'too_late'
    }
    replace(id, { text: e instanceof Error ? e.message : undoTooLate(), kind: 'alert', undo: undefined })
    return 'failed'
  }
  if (toast.counter) count(toast.counter, NOTICED_VERSION)
  replace(id, { text: toast.restored ?? '', kind: 'status', undo: undefined })
  if (!toast.restored) useToasts.getState().dismiss(id)
  return 'restored'
}

/** The rectangle of a box on screen. */
export interface Box {
  top: number
  bottom: number
  left: number
  right: number
}

export const TOAST_GAP = 8

/**
 * How far above the viewport's bottom the toast sits so that its box never covers the focused field:
 * the resting offset, or just above the field when the field would otherwise be covered.
 */
export function toastBottom(resting: number, toastHeight: number, viewportHeight: number, field: Box | null): number {
  if (!field) return resting
  const top = viewportHeight - resting - toastHeight
  const bottom = viewportHeight - resting
  const overlaps = field.bottom > top && field.top < bottom
  if (!overlaps) return resting
  return Math.max(resting, viewportHeight - field.top + TOAST_GAP)
}
