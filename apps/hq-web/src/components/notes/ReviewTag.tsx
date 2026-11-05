import { t } from '~/strings'
import type { ReviewTagKind } from '~/lib/noteReview'

const WORDS = {
  unreviewed: 'note_review_unreviewed',
  accepted: 'note_review_accepted',
  reviewed: 'note_review_reviewed',
  disputed: 'note_review_disputed',
  superseded: 'note_review_superseded',
  unknown: 'note_review_unknown',
} as const

/** One tag, words always; only Something's wrong is amber. */
export function ReviewTag({ kind }: { kind: ReviewTagKind }) {
  return (
    <span
      className="a-body-sm-b"
      data-review={kind}
      style={{ color: kind === 'disputed' ? 'var(--warn)' : 'var(--ink-muted)' }}
    >
      {t(WORDS[kind])}
    </span>
  )
}
