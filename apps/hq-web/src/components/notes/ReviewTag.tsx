import { Check, CircleDashed, TriangleAlert } from 'lucide-react'
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

const LOOK = {
  unreviewed: { cls: 'dash', Icon: CircleDashed },
  accepted: { cls: 'ok', Icon: Check },
  reviewed: { cls: 'ok', Icon: Check },
  disputed: { cls: 'warn', Icon: TriangleAlert },
  superseded: { cls: 'muted', Icon: CircleDashed },
  unknown: { cls: 'muted', Icon: CircleDashed },
} as const

/** One tag, words always; only Something's wrong is amber, and an unchecked note is dashed. */
export function ReviewTag({ kind }: { kind: ReviewTagKind }) {
  const { cls, Icon } = LOOK[kind]
  return (
    <span className={`an-rv ${cls}`} data-review={kind}>
      <Icon className="a-ic" aria-hidden="true" />
      {t(WORDS[kind])}
    </span>
  )
}
