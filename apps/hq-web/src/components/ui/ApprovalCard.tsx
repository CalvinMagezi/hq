import type { ReactNode } from 'react'
import { t } from '~/strings'

const QUOTE_LIMIT = 600

/** The arguments as one line the owner can read: `{"path": "Projects/Old plan.md"}`. */
export function quoteArgs(args: Record<string, unknown> | null | undefined): string {
  if (!args) return ''
  const parts = Object.entries(args).map(([k, v]) => `${JSON.stringify(k)}: ${JSON.stringify(v)}`)
  return `{${parts.join(', ')}}`
}

/** The label of the Allow button: "Allow once", or "Allow delete" when allowing loses something. */
export function allowLabel(loses: boolean | undefined, verb: string | null | undefined): string {
  return loses && verb ? t('approval_allow_verb', { verb }) : t('notice_action_allow_once')
}

/** A request that loses something never gets the green button. */
export function allowTone(loses: boolean | undefined): 'danger' | 'ask' {
  return loses ? 'danger' : 'ask'
}

export interface ApprovalCardProps {
  /** The tool name, when the server recorded a tool call. */
  tool?: string | null
  /** The arguments, or the command text when the request is a shell command. */
  args?: Record<string, unknown> | null
  command?: string | null
  /** What the quote is headed with, for example approval_quote_chat. */
  quoteTitle: string
  /** HQ's own explanation, always below the quote and marked as HQ's words. */
  inWords?: string | null
  loses?: boolean
  verb?: string | null
  /** False when only the agent's own page can answer: no Allow button. */
  canAllow?: boolean
  busy?: boolean
  error?: string | null
  onDecline: () => void
  onAllow: () => void
  /** Where the other answers live. */
  more?: ReactNode
}

/** One approval: the exact tool call first, HQ's words after, Decline left of Allow. */
export function ApprovalCard(p: ApprovalCardProps) {
  const raw = p.command ?? quoteArgs(p.args)
  const cut = raw.length > QUOTE_LIMIT
  const shown = cut ? raw.slice(0, QUOTE_LIMIT) : raw
  const allow = allowLabel(p.loses, p.verb)
  return (
    <section className="a-card" aria-label={t('approval_head', { who: t('approval_who_hq') })}>
      <p className="text-xs font-semibold">{p.quoteTitle}</p>
      <blockquote className="a-quote" data-testid="approval-quote">
        {p.tool && <code data-testid="approval-tool">{p.tool}</code>}
        {shown && <pre className="whitespace-pre-wrap break-words">{shown}</pre>}
        {!p.tool && !shown && <span>{t('approval_not_read')}</span>}
        {cut && <small>{t('approval_more')}</small>}
      </blockquote>
      {p.inWords && (
        <>
          <p className="text-xs font-semibold">{t('approval_in_words')}</p>
          <p className="text-xs">{p.inWords}</p>
        </>
      )}
      <div className="a-dlg-actions" style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        <button type="button" className="a-btn secondary" disabled={p.busy} onClick={p.onDecline}>
          {t('notice_action_decline')}
        </button>
        {p.canAllow !== false && (
          <button
            type="button"
            className={allowTone(p.loses) === 'danger' ? 'a-btn danger' : 'a-btn'}
            disabled={p.busy}
            aria-busy={p.busy || undefined}
            onClick={p.onAllow}
          >
            {p.busy ? t('approval_answering') : allow}
          </button>
        )}
        {p.more}
      </div>
      {p.error && <p role="alert">{p.error}</p>}
    </section>
  )
}
