import { useState } from 'react'
import { Check, Loader2, MinusCircle, SkipForward, ThumbsDown, ThumbsUp, X } from 'lucide-react'
import { answerAudit, fetchAudits, type AuditCard, type AuditStack } from '~/lib/verificationApi'
import { usePolled } from '../../sessions/usePolled'
import { SHEET_DANGER, SHEET_PRIMARY, SHEET_QUIET, Sheet } from './Sheet'
import { auditEstimate, criteriaSummary, levelLabel } from './verification'

/** Facts shown on a card; the rest are one tap away in the task. */
const FACTS_SHOWN = 8
const MORE = 10
const IDLE_POLL_MS = 60_000

function ResultIcon({ result }: { result: 'pass' | 'fail' | 'unclear' | boolean | null }) {
  if (result === 'pass' || result === true) return <Check className="w-3.5 h-3.5 text-emerald-400 shrink-0 mt-0.5" aria-label="met" />
  if (result === 'fail' || result === false) return <X className="w-3.5 h-3.5 text-rose-400 shrink-0 mt-0.5" aria-label="not met" />
  return <MinusCircle className="w-3.5 h-3.5 text-neutral-500 shrink-0 mt-0.5" aria-label="not decided" />
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
    <article className="space-y-3" aria-label="Audit">
      <div>
        <p className="text-sm font-semibold text-white break-words">
          {card.task ? (
            <>
              {card.task.display_id} <span className="font-normal text-neutral-200">{card.task.title}</span>
            </>
          ) : (
            'A task that no longer exists'
          )}
        </p>
        {v && (
          <p className="text-xs text-neutral-400 mt-0.5">
            Completed on {levelLabel(v.level)}, tier {v.tier}, by {v.verifier}
            {v.independence !== 'none' ? ` (${v.independence.replaceAll('_', ' ')})` : ''}. Artifact: {v.artifact}.
          </p>
        )}
      </div>

      {v && (
        <div>
          <p className="text-[11px] font-semibold text-neutral-500 uppercase tracking-wider mb-1">{criteriaSummary(v.criteria)}</p>
          <ul className="space-y-1">
            {v.criteria.map((c) => (
              <li key={c.text} className="flex items-start gap-1.5 text-xs text-neutral-200">
                <ResultIcon result={c.result} />
                <span className="min-w-0 break-words">
                  {c.text}
                  {c.note ? <span className="text-neutral-500"> · {c.note}</span> : null}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {facts.length > 0 && (
        <div>
          <p className="text-[11px] font-semibold text-neutral-500 uppercase tracking-wider mb-1">What HQ saw</p>
          <ul className="space-y-1">
            {facts.map((o) => (
              <li key={o.id} className="flex items-start gap-1.5 text-xs text-neutral-400">
                <ResultIcon result={o.ok} />
                <span className="min-w-0 break-words">{o.fact}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {card.links.length > 0 && (
        <p className="text-xs text-neutral-500 break-all">
          Linked: {card.links.map((l) => `${l.kind} ${l.ref}`).join(', ')}
        </p>
      )}

      {disagreeing ? (
        <div className="space-y-2">
          <label className="block text-xs text-neutral-300">
            What is wrong? It goes on the task, which is reopened.
            <textarea
              value={note}
              onChange={(e) => onNote(e.target.value)}
              rows={3}
              className="mt-1 w-full px-3 py-2 rounded-xl text-sm text-neutral-100 hq-field focus:outline-none focus:ring-1 focus:ring-rose-400"
            />
          </label>
          <div className="flex gap-2">
            <button type="button" disabled={busy} onClick={onCancelDisagree} className={`${SHEET_QUIET} flex-1`}>
              Back
            </button>
            <button type="button" disabled={busy} onClick={onDisagree} className={`${SHEET_DANGER} flex-1`}>
              <ThumbsDown className="w-4 h-4" />
              Reopen it
            </button>
          </div>
        </div>
      ) : (
        <div className="flex gap-2">
          <button type="button" disabled={busy} onClick={onAgree} className={`${SHEET_PRIMARY} flex-1`}>
            <ThumbsUp className="w-4 h-4" />
            Agree
          </button>
          <button type="button" disabled={busy} onClick={onDisagree} className={`${SHEET_DANGER} flex-1`}>
            <ThumbsDown className="w-4 h-4" />
            Disagree
          </button>
          <button type="button" disabled={busy} onClick={onSkip} className={`${SHEET_QUIET} px-3`} aria-label="Skip">
            <SkipForward className="w-4 h-4" />
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
    <div className="space-y-4">
      <p className="text-xs text-neutral-400">
        You are checking HQ, not the work: do you agree that these were done? Every disagreement reopens the task and counts against the verifier.
      </p>
      {card ? (
        <>
          <p className="text-[11px] text-neutral-500">
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
        <p className="text-sm text-neutral-200">Nothing left to answer. Ask for more whenever you like: the ones with the least data come first.</p>
      )}
      {error && <p className="text-xs text-rose-300">{error}</p>}
      <div className="pt-3 border-t border-white/10 space-y-2.5">
        <p className="text-xs text-neutral-400" aria-live="polite">
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
  const [extra, setExtra] = useState<AuditStack | null>(null)
  if (!open) return null

  const base = extra ?? stack.data
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
      setExtra(await fetchAudits(0))
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
              setExtra(await fetchAudits(MORE))
            })
          }
        />
      ) : (
        <p className="text-sm text-neutral-400 flex items-center gap-2">
          <Loader2 className="w-4 h-4 animate-spin" />
          {stack.error ?? 'Loading the audit'}
        </p>
      )}
    </Sheet>
  )
}
