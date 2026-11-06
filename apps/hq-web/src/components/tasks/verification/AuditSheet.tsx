import { useEffect, useState } from 'react'
import { Check, Loader2, MinusCircle, SkipForward, ThumbsDown, ThumbsUp, TriangleAlert } from 'lucide-react'
import { t } from '~/strings'
import '../leaf.css'
import { answerAudit, fetchAudits, type AuditCard, type AuditStack } from '~/lib/verificationApi'
import { usePolled } from '../../sessions/usePolled'
import { SHEET_DANGER, SHEET_PRIMARY, SHEET_QUIET, Sheet } from './Sheet'
import { auditEstimate, criteriaSummary, kindLabel, levelLabel } from './verification'

/** Facts shown on a card; the rest are one tap away in the task. */
const FACTS_SHOWN = 8
const MORE = 10
const IDLE_POLL_MS = 60_000

function ResultIcon({ result }: { result: 'pass' | 'fail' | 'unclear' | boolean | null }) {
  if (result === 'pass' || result === true) return <Check className="a-ic xs" aria-label={t('verify_met')} />
  if (result === 'fail' || result === false) return <TriangleAlert className="a-ic xs a-warn-t" aria-label={t('verify_not_met')} />
  return <MinusCircle className="a-ic xs a-muted" aria-label={t('verify_not_decided')} />
}

interface CardProps {
  card: AuditCard
  busy: boolean
  disagreeing: boolean
  note: string
  onNote: (v: string) => void
  onAgree: () => void
  onDisagree: () => void
  onCancelDisagree: () => void
  onSkip: () => void
}

/** One completed task: what it was, how HQ decided, and the one tap that answers it. */
export function AuditCardView({ card, busy, disagreeing, note, onNote, onAgree, onDisagree, onCancelDisagree, onSkip }: CardProps) {
  const v = card.verdict
  const facts = card.observations.filter((o) => o.kind !== 'contract').slice(0, FACTS_SHOWN)
  return (
    <article className="a-card" aria-label="Audit">
      <div className="l-stack">
        <p className="a-body-sm-b l-break">
          {card.task ? (
            <>
              {card.task.display_id} <span>{card.task.title}</span>
            </>
          ) : (
            'A task that no longer exists'
          )}
        </p>
        {v && (
          <p className="a-cap a-muted">
            {t('check_audit_done_line', {
              level: levelLabel(v.level),
              kind: kindLabel(v.tier),
              verifier: v.verifier,
              how: v.independence !== 'none' ? ` (${v.independence.replaceAll('_', ' ')})` : '',
            })}
          </p>
        )}
      </div>

      {v && (
        <div className="l-stack">
          <p className="a-label">{criteriaSummary(v.criteria)}</p>
          <ul className="l-stack">
            {v.criteria.map((c) => (
              <li key={c.text} className="l-row a-body-sm">
                <ResultIcon result={c.result} />
                <span className="l-grow l-break">
                  {c.text}
                  {c.note ? <span className="a-muted"> · {c.note}</span> : null}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {facts.length > 0 && (
        <div className="l-stack">
          <p className="a-label">{t('verify_audit_saw')}</p>
          <ul className="l-stack">
            {facts.map((o) => (
              <li key={o.id} className="l-row a-cap a-muted">
                <ResultIcon result={o.ok} />
                <span className="l-grow l-break">{o.fact}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {card.links.length > 0 && (
        <p className="a-cap a-muted l-break">
          Linked: {card.links.map((l) => `${l.kind} ${l.ref}`).join(', ')}
        </p>
      )}

      {disagreeing ? (
        <div className="l-stack">
          <label className="a-field-label block">
            {t('verify_audit_wrong')}
            <textarea
              value={note}
              onChange={(e) => onNote(e.target.value)}
              rows={3}
              className="a-input mt-1"
            />
          </label>
          <div className="l-bar">
            <button type="button" disabled={busy} onClick={onCancelDisagree} className={`${SHEET_QUIET} flex-1`}>
              {t('verify_audit_back')}
            </button>
            <button type="button" disabled={busy} onClick={onDisagree} className={`${SHEET_DANGER} flex-1`}>
              <ThumbsDown className="a-ic sm" aria-hidden="true" />
              {t('verify_audit_reopen')}
            </button>
          </div>
        </div>
      ) : (
        <div className="l-bar">
          <button type="button" disabled={busy} onClick={onAgree} className={`${SHEET_PRIMARY} flex-1`}>
            <ThumbsUp className="a-ic sm" aria-hidden="true" />
            {t('verify_audit_agree')}
          </button>
          <button type="button" disabled={busy} onClick={onDisagree} className={`${SHEET_DANGER} flex-1`}>
            <ThumbsDown className="a-ic sm" aria-hidden="true" />
            {t('verify_audit_disagree')}
          </button>
          <button type="button" disabled={busy} onClick={onSkip} className={`${SHEET_QUIET} px-3`} aria-label={t('verify_audit_skip')}>
            <SkipForward className="a-ic sm" aria-hidden="true" />
          </button>
        </div>
      )}
    </article>
  )
}

interface StackProps {
  stack: AuditStack
  /** Which card, counting from the first not yet answered or skipped. */
  index: number
  busy: boolean
  error: string | null
  disagreeing: boolean
  note: string
  onNote: (v: string) => void
  onAgree: () => void
  onDisagree: () => void
  onCancelDisagree: () => void
  onSkip: () => void
  onMore: () => void
}

export function AuditStackView({ stack, index, busy, error, disagreeing, note, onNote, onAgree, onDisagree, onCancelDisagree, onSkip, onMore }: StackProps) {
  const card = stack.cards[index]
  return (
    <div className="l-stack">
      <p className="a-body-sm a-muted">
        {t('verify_audit_intro')}
      </p>
      {card ? (
        <>
          <p className="a-cap a-muted">
            {index + 1} of {stack.cards.length}
          </p>
          <AuditCardView
            card={card}
            busy={busy}
            disagreeing={disagreeing}
            note={note}
            onNote={onNote}
            onAgree={onAgree}
            onDisagree={onDisagree}
            onCancelDisagree={onCancelDisagree}
            onSkip={onSkip}
          />
        </>
      ) : (
        <p className="a-body-sm">{t('verify_audit_none_left')}</p>
      )}
      {error && <p role="alert" className="a-why">{error}</p>}
      <div className="l-stack">
        <p className="a-body-sm a-muted" aria-live="polite">
          {auditEstimate(stack.calibration)}
        </p>
        <button type="button" disabled={busy} onClick={onMore} className={`${SHEET_QUIET} w-full`}>
          Audit {MORE} more
        </button>
      </div>
    </div>
  )
}

interface Props {
  open: boolean
  onClose: () => void
}

const errorText = (e: unknown) => (e instanceof Error ? e.message : 'Something went wrong')

/** The audit view: a stack of cards, one tap each, and a running estimate of how often HQ is wrong. */
export function AuditSheet({ open, onClose }: Props) {
  const stack = usePolled('verification-audits', () => fetchAudits(0), IDLE_POLL_MS, open)
  const [skipped, setSkipped] = useState<ReadonlySet<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [disagreeing, setDisagreeing] = useState(false)
  const [note, setNote] = useState('')
  // What was skipped, half-written or failed belongs to one sitting: a sheet that is closed and
  // opened again starts clean, on the server's current open list.
  useEffect(() => {
    if (open) return
    setSkipped(new Set())
    setBusy(false)
    setError(null)
    setDisagreeing(false)
    setNote('')
  }, [open])
  if (!open) return null

  // The server's open list is the only source: answering or drawing more refreshes it, so a card
  // answered on another device, or a weekly push that arrived meanwhile, shows up here too.
  const base = stack.data
  // Skipped cards go to the back; answered ones are gone from the server's open list.
  const cards = base ? [...base.cards.filter((c) => !skipped.has(c.audit.id)), ...base.cards.filter((c) => skipped.has(c.audit.id))] : []
  const view: AuditStack | null = base ? { ...base, cards } : null

  const guard = async (fn: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try {
      await fn()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const answer = (a: 'agree' | 'disagree') =>
    guard(async () => {
      const card = cards[0]
      if (!card) return
      await answerAudit(card.audit.id, a, a === 'disagree' ? note : '')
      setNote('')
      setDisagreeing(false)
      await stack.refresh()
    })

  return (
    <Sheet title="Audit" onClose={onClose}>
      {view ? (
        <AuditStackView
          stack={view}
          index={0}
          busy={busy}
          error={error ?? stack.error}
          disagreeing={disagreeing}
          note={note}
          onNote={setNote}
          onAgree={() => answer('agree')}
          onDisagree={() => (disagreeing ? answer('disagree') : setDisagreeing(true))}
          onCancelDisagree={() => setDisagreeing(false)}
          onSkip={() => {
            const id = cards[0]?.audit.id
            if (id) setSkipped((prev) => new Set(prev).add(id))
            setDisagreeing(false)
          }}
          onMore={() =>
            guard(async () => {
              await fetchAudits(MORE)
              await stack.refresh()
            })
          }
        />
      ) : (
        <p className="a-loading">
          <Loader2 className="a-ic sm animate-spin" aria-hidden="true" />
          {stack.error ?? 'Loading the audit'}
        </p>
      )}
    </Sheet>
  )
}
