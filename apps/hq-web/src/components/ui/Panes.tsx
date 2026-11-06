import type { ReactNode } from 'react'

export interface PanesProps {
  /** The list pane (360 from 1024 px); omit for a place that has none. */
  list?: ReactNode
  /** The open item. Below 1024 px it replaces the list while `showDetail` is set. */
  detail: ReactNode
  /** Details for the open item: docked at 320 from 1400 px, a side sheet over the detail below that while `sideOpen`. */
  side?: ReactNode
  showDetail?: boolean
  sideOpen?: boolean
  /** Accessible names for the panes. */
  listLabel?: string
  detailLabel?: string
  sideLabel?: string
}

/** The class list for `.a-panes`, the design's one layout for a list and its open item. */
export function panesClass({ list, side, showDetail, sideOpen }: Pick<PanesProps, 'list' | 'side' | 'showDetail' | 'sideOpen'>): string {
  return [
    'a-panes hq-panes',
    list === undefined ? 'no-list' : '',
    side === undefined ? '' : 'has-side',
    showDetail ? 'is-detail' : '',
    sideOpen ? 'side-open' : '',
  ]
    .filter(Boolean)
    .join(' ')
}

/** A list, the open item and its details at every width (AlignedShell 2), from the design's pane classes. */
export function Panes({ list, detail, side, showDetail, sideOpen, listLabel, detailLabel, sideLabel }: PanesProps) {
  return (
    <div className={panesClass({ list, side, showDetail, sideOpen })}>
      {list !== undefined && (
        <section className="a-pane a-pane-list" aria-label={listLabel}>
          {list}
        </section>
      )}
      <section className="a-pane a-pane-detail" aria-label={detailLabel}>
        {detail}
      </section>
      {side !== undefined && (
        <aside className="a-pane a-pane-side" aria-label={sideLabel}>
          {side}
        </aside>
      )}
    </div>
  )
}

/** A pane's head row: back (below 1024 px), the title, then its own buttons. */
export function PaneHead({ title, back, children }: { title: ReactNode; back?: ReactNode; children?: ReactNode }) {
  return (
    <div className="a-pane-head">
      {back}
      <h2 className="a-h3">{title}</h2>
      {children}
    </div>
  )
}
