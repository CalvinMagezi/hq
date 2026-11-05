import { t } from '~/strings'
import { relTime } from '~/lib/time'
import { historyOrder, reviewTag, whoWrote, type NoteVersion, type NoteVersions, type WhoKind } from '~/lib/noteReview'
import { ReviewTag } from './ReviewTag'

export function whoText(who: WhoKind): string {
  switch (who.kind) {
    case 'you':
      return t('note_who_you')
    case 'hq':
      return t('note_who_hq')
    case 'imported':
      return t('note_who_imported')
    case 'helper':
      return t('note_who_helper', { name: who.name })
    default:
      return t('note_who_unknown')
  }
}

/** The label of a row: Now for the newest, First for the oldest, Before between. */
export function historyLabel(index: number, total: number): string {
  if (index === 0) return t('note_history_now')
  return index === total - 1 ? t('note_history_first') : t('note_history_before')
}

/** Every recorded version, newest first: who, when and where its review stands. */
export function NoteHistory({ versions }: { versions: NoteVersions }) {
  const rows: NoteVersion[] = historyOrder(versions)
  return (
    <ol className="a-body-sm" style={{ display: 'flex', flexDirection: 'column', gap: 8, listStyle: 'none', padding: 0, margin: 0 }}>
      {rows.map((v, i) => (
        <li key={v.id} style={{ display: 'flex', flexWrap: 'wrap', gap: 8, alignItems: 'baseline' }}>
          <span>
            {t('note_history_line', { label: historyLabel(i, rows.length), who: whoText(whoWrote(v)), when: relTime(v.written_at) })}
          </span>
          <ReviewTag kind={reviewTag(v)} />
          {v.review_state === 'disputed' && v.review_reason && <q className="a-muted">{v.review_reason}</q>}
        </li>
      ))}
    </ol>
  )
}
