import { useEffect, useId, useRef } from 'react'
import { t } from '~/strings'

export type DialogKind = 'ask' | 'red' | 'spend'

const MAX_WIDTH = 420
const SIDE_MARGIN = 24
const PAD = 24
const GAP = 8
const CHAR_PX = 8
const BUTTON_CHROME = 50
const BAD_RED_LABELS = ['yes', 'ok', 'okay', 'continue', 'confirm']

/** Whether the two buttons fit on one row at this viewport width, each at the same width. */
export function fitsSideBySide(viewportWidth: number, keepLabel: string, actLabel: string): boolean {
  const inner = Math.min(MAX_WIDTH, viewportWidth - SIDE_MARGIN * 2) - PAD * 2
  const widest = Math.max(keepLabel.length, actLabel.length) * CHAR_PX + BUTTON_CHROME
  return widest * 2 + GAP <= inner
}

/** A red dialog's action names what is lost: it starts with the title's verb and is never a bare Yes or OK. */
export function redLabelProblem(title: string, act: string): string | null {
  const label = act.trim().toLowerCase()
  if (BAD_RED_LABELS.includes(label)) return 'bare label'
  const verb = title.trim().split(/\s+/)[0]?.toLowerCase()
  if (!verb || !label.startsWith(verb)) return 'does not start with the title verb'
  return null
}

export type DismissWay = 'esc' | 'scrim' | 'back'

/** Esc, a tap on the scrim and Back all mean keep; none of them ever act, and none works while busy. */
export function dismissOutcome(_way: DismissWay, busy: boolean): 'keep' | 'ignore' {
  return busy ? 'ignore' : 'keep'
}

export interface DialogProps {
  kind: DialogKind
  title: string
  body: string
  keepLabel: string
  actLabel: string
  busyLabel?: string
  busy?: boolean
  error?: string | null
  /** The one dialog with three choices: discarding unsaved note changes. */
  third?: { label: string; onPick: () => void }
  onKeep: () => void
  onAct: () => void
  /** Test and layout seam; the window's width when not given. */
  viewportWidth?: number
}

/** The one dialog for every red, ask and spend question, built on the native <dialog>. */
export function Dialog(props: DialogProps) {
  const { kind, title, body, keepLabel, actLabel, busyLabel, busy, error, third, onKeep, onAct } = props
  const ref = useRef<HTMLDialogElement>(null)
  const titleId = useId()
  const bodyId = useId()
  const busyRef = useRef(!!busy)
  const keepRef = useRef(onKeep)
  busyRef.current = !!busy
  keepRef.current = onKeep
  const width = props.viewportWidth ?? (typeof window === 'undefined' ? 390 : window.innerWidth)
  const side = fitsSideBySide(width, keepLabel, actLabel)

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const trigger = document.activeElement as HTMLElement | null
    if (!el.open) el.showModal()
    el.querySelector<HTMLElement>('[data-dialog-title]')?.focus()
    window.history.pushState({ hqDialog: true }, '')
    let closedByBack = false
    const back = () => {
      closedByBack = true
      if (dismissOutcome('back', busyRef.current) === 'keep') keepRef.current()
    }
    window.addEventListener('popstate', back)
    return () => {
      window.removeEventListener('popstate', back)
      if (!closedByBack && window.history.state?.hqDialog) window.history.back()
      if (el.open) el.close()
      trigger?.focus?.()
    }
  }, [])

  const keep = (
    <button type="button" className="a-btn secondary" disabled={busy} onClick={onKeep} style={{ flex: side ? 1 : undefined }}>
      {keepLabel}
    </button>
  )
  const act = (
    <button
      type="button"
      className={kind === 'red' ? 'a-btn danger' : kind === 'ask' ? 'a-btn' : 'a-btn secondary'}
      disabled={busy}
      aria-busy={busy || undefined}
      onClick={onAct}
      style={{ flex: side ? 1 : undefined }}
    >
      {busy && busyLabel ? busyLabel : actLabel}
    </button>
  )

  return (
    <dialog
      ref={ref}
      className="a-dlg backdrop:bg-black/[.62]"
      data-kind={kind}
      data-layout={side ? 'side' : 'stacked'}
      aria-labelledby={titleId}
      aria-describedby={bodyId}
      style={{ position: 'fixed', inset: 0, margin: 'auto', padding: 0, width: `min(${MAX_WIDTH}px, 100% - ${SIDE_MARGIN * 2}px)`, height: 'fit-content', right: 'auto', bottom: 'auto', left: 'auto', top: 'auto' }}
      onCancel={(e) => {
        e.preventDefault()
        if (dismissOutcome('esc', busyRef.current) === 'keep') onKeep()
      }}
      onClick={(e) => {
        if (e.target === ref.current && dismissOutcome('scrim', busyRef.current) === 'keep') onKeep()
      }}
    >
      <div style={{ padding: PAD }}>
        <h2 id={titleId} data-dialog-title tabIndex={-1} style={{ outline: 'none' }}>
          {title}
        </h2>
        <p id={bodyId} className="whitespace-pre-line">
          {body}
        </p>
        {error && <p role="alert">{error}</p>}
        <div className="a-dlg-actions" style={{ display: 'flex', flexDirection: side ? 'row' : 'column', gap: GAP }}>
          {keep}
          {third && (
            <button type="button" className="a-btn secondary" disabled={busy} onClick={third.onPick}>
              {third.label}
            </button>
          )}
          {act}
        </div>
      </div>
    </dialog>
  )
}

/** The busy line a dialog's action shows while HQ works on it. */
export function workingLabel(): string {
  return t('confirm_busy_working')
}
