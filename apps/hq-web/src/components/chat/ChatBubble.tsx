import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { Mic, Copy, Check, Pencil, Plug, RotateCw, Terminal } from 'lucide-react'
import type { ChatOffer, ToolStep } from '~/store/threadStore'
import type { DriverMeta } from '~/lib/sessionsApi'
import type { ChatAttachment } from '~/lib/chatUploads'
import { CREDITS_TOOLTIP, formatCredits, totalCredits, type StepCredit } from '~/lib/stepCredits'
import { ToolCallBlock } from './ToolCallBlock'
import { ThinkingPanel } from './ThinkingPanel'
import { MessageAttachments } from './AttachmentChips'
import { canSchedule, scheduleChoices } from '~/lib/offersApi'
import { t } from '~/strings'
import { MarkdownViewer } from '../MarkdownViewer'

const COPIED_FLASH_MS = 1500
export const HQ_MARK = '/icons/hq-mark-96.png'

interface DisplayMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  timestamp: number
  toolSteps?: ToolStep[]
  stepCredits?: StepCredit[]
  reasoning?: string
  attachments?: ChatAttachment[]
  stopped?: boolean
  driver?: DriverMeta
  /** The MCP client that asked this question, when it did not come from this chat's composer. */
  viaMcp?: string
  /** Asked by voice. */
  spoken?: boolean
  offer?: ChatOffer
}

interface Props {
  message: DisplayMessage
  /** Still streaming: panels show progress and the footer waits for the end. */
  live?: boolean
  /** Offered on the user's own saved messages. */
  onEdit?: () => void
  /** Offered on the last reply. */
  onRegenerate?: () => void
  /** Answers the offer under this reply. */
  onOffer?: (yes: boolean) => void
  /** Brings the offer under this reply back at a time, in unix seconds. */
  onScheduleOffer?: (at: number) => void
}

/**
 * One chat message. A streaming reply renders through this same layout
 * (thinking, tools, then text), so nothing moves when it becomes a saved message.
 */
export function ChatBubble({ message, live = false, onEdit, onRegenerate, onOffer, onScheduleOffer }: Props) {
  const [copied, setCopied] = useState(false)
  const isUser = message.role === 'user'
  const steps = message.toolSteps ?? []
  const replyCredits = isUser ? null : totalCredits(message.stepCredits)
  const waiting = live && !message.content && !message.reasoning && steps.length === 0

  const handleCopy = () => {
    void navigator.clipboard.writeText(message.content)
    setCopied(true)
    setTimeout(() => setCopied(false), COPIED_FLASH_MS)
  }

  return (
    <div className={`${isUser ? 'a-u' : 'a-a'} hq-msg`}>
      {isUser && message.viaMcp && <McpLabel caller={message.viaMcp} />}
      {isUser && message.spoken && (
        <span className="a-cap a-muted hq-msg-label">
          <Mic className="a-ic xs" aria-hidden="true" /> {t('chat_spoken')}
        </span>
      )}
      {!isUser && message.driver && <DriverLabel driver={message.driver} />}
      {!isUser && message.reasoning && <ThinkingPanel content={message.reasoning} live={live} />}
      {!isUser && steps.length > 0 && <ToolCallBlock steps={steps} credits={message.stepCredits} live={live} />}
      {message.attachments && <MessageAttachments attachments={message.attachments} />}

      {message.content && (isUser ? <p className="hq-msg-text">{message.content}</p> : <MarkdownViewer content={message.content} bare />)}
      {waiting && (
        <p className="a-body-sm a-muted hq-msg-label">
          <span className="a-spin" aria-hidden="true" />
          {t('chat_working')}
        </p>
      )}
      {!isUser && !live && message.offer && onOffer && <OfferLine offer={message.offer} onAnswer={onOffer} onSchedule={onScheduleOffer} />}

      {!live && (
        <div className="a-meta">
          <span>
            {new Date(message.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
            {message.stopped && <span className="a-warn-t hq-meta-gap">{t('chat_stopped')}</span>}
            {replyCredits !== null && (
              <span className="hq-meta-gap" title={CREDITS_TOOLTIP}>
                {formatCredits(replyCredits)}
              </span>
            )}
          </span>
          <span className="hq-msg-actions">
            {onEdit && (
              <button type="button" onClick={onEdit} className="a-ib sm" title={t('chat_edit_resend')} aria-label={t('chat_edit_resend')}>
                <Pencil className="a-ic xs" aria-hidden="true" />
              </button>
            )}
            {onRegenerate && (
              <button type="button" onClick={onRegenerate} className="a-ib sm" title={t('chat_regenerate')} aria-label={t('chat_regenerate')}>
                <RotateCw className="a-ic xs" aria-hidden="true" />
              </button>
            )}
            {message.content && (
              <button type="button" onClick={handleCopy} className="a-ib sm" title={copied ? t('chat_copied') : t('chat_copy')} aria-label={t('chat_copy')}>
                {copied ? <Check className="a-ic xs a-ok-t" aria-hidden="true" /> : <Copy className="a-ic xs" aria-hidden="true" />}
              </button>
            )}
          </span>
        </div>
      )}
    </div>
  )
}

/** One thing HQ can also do here, with Yes and Not now. Not a notification: it sits under the reply. */
function OfferLine({ offer, onAnswer, onSchedule }: { offer: ChatOffer; onAnswer: (yes: boolean) => void; onSchedule?: (at: number) => void }) {
  const [picking, setPicking] = useState(false)
  if (picking && onSchedule) {
    return (
      <div className="hq-offer">
        <span className="a-body-sm a-muted">{offer.line}</span>
        <span className="a-btns">
          {scheduleChoices(new Date()).map((c) => (
            <button key={c.key} type="button" className="a-btn secondary compact" onClick={() => onSchedule(c.at)}>
              {t(c.key)}
            </button>
          ))}
          <button type="button" className="a-btn quiet compact" onClick={() => setPicking(false)}>
            {t('chat_offer_back')}
          </button>
        </span>
      </div>
    )
  }
  return (
    <div className="hq-offer">
      <span className="a-body-sm a-muted">{offer.line}</span>
      <span className="a-btns">
        <button type="button" className="a-btn secondary compact" onClick={() => onAnswer(true)}>
          {t('chat_offer_yes')}
        </button>
        {onSchedule && canSchedule(offer) && (
          <button type="button" className="a-btn secondary compact" onClick={() => setPicking(true)}>
            {t('chat_offer_schedule')}
          </button>
        )}
        <button type="button" className="a-btn quiet compact" onClick={() => onAnswer(false)}>
          {t('chat_offer_not_now')}
        </button>
      </span>
    </div>
  )
}

/** Marks a question an outside MCP client asked HQ, so it is not mistaken for something typed here. */
function McpLabel({ caller }: { caller: string }) {
  return (
    <div className="a-cap a-muted hq-msg-label" title={t('chat_via_mcp_title')}>
      <Plug className="a-ic xs" aria-hidden="true" />
      <span>{t('chat_via_mcp', { caller })}</span>
    </div>
  )
}

/** Marks a message HQ posted for a watched session rather than in answer to the user. */
function DriverLabel({ driver }: { driver: DriverMeta }) {
  const drove = driver.mode === 'drive'
  return (
    <div className="a-cap a-muted hq-msg-label" title={drove ? t('chat_helper_acted') : t('chat_helper_update')}>
      <Terminal className="a-ic xs" aria-hidden="true" />
      <span>
        {t('chat_helper_label', { id: driver.sessionId })}
        {driver.reason && ` · ${driver.reason}`}
      </span>
      {drove && <span className="a-warn-t">{t('chat_acted_for_you')}</span>}
      {!drove && (
        <Link to="/helpers/$id" params={{ id: driver.sessionId }} className="a-link">
          {t('chat_answer_in_helpers')}
        </Link>
      )}
    </div>
  )
}
