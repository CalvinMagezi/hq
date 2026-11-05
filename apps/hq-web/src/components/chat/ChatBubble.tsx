import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { Mic, Copy, Check, Pencil, Plug, RotateCw, Terminal, User } from 'lucide-react'
import type { ChatOffer, ToolStep } from '~/store/threadStore'
import type { DriverMeta } from '~/lib/sessionsApi'
import type { ChatAttachment } from '~/lib/chatUploads'
import { CREDITS_TOOLTIP, formatCredits, totalCredits, type StepCredit } from '~/lib/stepCredits'
import { ToolCallBlock } from './ToolCallBlock'
import { ThinkingPanel } from './ThinkingPanel'
import { MessageAttachments } from './AttachmentChips'
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
}

// Hover reveals actions on a desktop; a touch screen has no hover, so they always show there.
const ACTION_BUTTON =
  'p-1.5 -m-1 rounded hover:bg-white/10 transition-colors opacity-0 group-hover:opacity-100 [@media(hover:none)]:opacity-100 flex items-center gap-1'

/**
 * One chat message. A streaming reply renders through this same layout
 * (thinking, tools, then text), so nothing moves when it becomes a saved message.
 */
export function ChatBubble({ message, live = false, onEdit, onRegenerate, onOffer }: Props) {
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
    <div className={`flex items-start gap-2.5 mb-4 ${isUser ? 'flex-row-reverse' : 'flex-row'}`}>
      <div
        className="hidden sm:flex w-7 h-7 rounded-lg items-center justify-center shrink-0 text-xs font-bold"
        style={{
          background: isUser ? 'rgba(0, 255, 163, 0.15)' : 'rgba(0, 173, 238, 0.15)',
          border: isUser ? '1px solid rgba(0, 255, 163, 0.3)' : '1px solid rgba(0, 173, 238, 0.3)',
          color: isUser ? 'var(--accent-green)' : 'var(--accent-blue)',
        }}
      >
        {isUser ? <User className="w-3.5 h-3.5" /> : <img src={HQ_MARK} alt="HQ" className="w-5 h-5 object-contain" />}
      </div>

      <div
        className={`group relative rounded-2xl px-3.5 sm:px-4 py-3 text-[15px] leading-relaxed min-w-0 ${isUser ? 'max-w-[85%] sm:max-w-[75%]' : 'flex-1 max-w-full'}`}
        style={
          isUser
            ? {
                background: 'rgba(255, 255, 255, 0.07)',
                border: '1px solid rgba(255, 255, 255, 0.11)',
                boxShadow: 'inset 0 1px 0 rgba(255, 255, 255, 0.09)',
                backdropFilter: 'blur(16px)',
                borderRadius: '20px 6px 20px 20px',
                color: 'var(--text-primary)',
              }
            : {
                background: 'rgba(255, 255, 255, 0.025)',
                border: '1px solid rgba(255, 255, 255, 0.07)',
                borderRadius: '6px 20px 20px 20px',
                color: 'var(--text-primary)',
              }
        }
      >
        {isUser && message.viaMcp && <McpLabel caller={message.viaMcp} />}
        {isUser && message.spoken && (
          <span className="flex items-center gap-1 text-[11px] mb-1" style={{ color: 'var(--ink-muted)' }}>
            <Mic className="w-3 h-3" aria-hidden="true" /> {t('chat_spoken')}
          </span>
        )}
        {!isUser && message.driver && <DriverLabel driver={message.driver} />}
        {!isUser && message.reasoning && <ThinkingPanel content={message.reasoning} live={live} />}
        {!isUser && steps.length > 0 && <ToolCallBlock steps={steps} credits={message.stepCredits} live={live} />}
        {message.attachments && <MessageAttachments attachments={message.attachments} />}

        {isUser ? (
          message.content && <p className="whitespace-pre-wrap break-words">{message.content}</p>
        ) : (
          message.content && <MarkdownViewer content={message.content} bare />
        )}
        {waiting && <div className="text-xs text-neutral-500 animate-pulse">{t('chat_working')}</div>}
        {!isUser && !live && message.offer && onOffer && <OfferLine offer={message.offer} onAnswer={onOffer} />}

        {!live && (
          <div className="flex items-center justify-between mt-1.5 pt-1 border-t border-white/5 opacity-60 text-xs " style={{ color: 'var(--text-dim)' }}>
            <span>
              {new Date(message.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
              {message.stopped && <span className="ml-2" style={{ color: 'var(--accent-amber)' }}>{t('chat_stopped')}</span>}
              {replyCredits !== null && (
                <span className="ml-2" title={CREDITS_TOOLTIP}>{formatCredits(replyCredits)}</span>
              )}
            </span>
            <span className="flex items-center gap-3">
              {onEdit && (
                <button type="button" onClick={onEdit} className={ACTION_BUTTON} title={t('chat_edit_resend')} aria-label={t('chat_edit_resend')}>
                  <Pencil className="w-3.5 h-3.5" />
                </button>
              )}
              {onRegenerate && (
                <button type="button" onClick={onRegenerate} className={ACTION_BUTTON} title={t('chat_regenerate')} aria-label={t('chat_regenerate')}>
                  <RotateCw className="w-3.5 h-3.5" />
                </button>
              )}
              {message.content && (
                <button type="button" onClick={handleCopy} className={ACTION_BUTTON} title={copied ? t('chat_copied') : t('chat_copy')} aria-label={t('chat_copy')}>
                  {copied ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
                </button>
              )}
            </span>
          </div>
        )}
      </div>
    </div>
  )
}

/** One thing HQ can also do here, with Yes and Not now. Not a notification: it sits under the reply. */
function OfferLine({ offer, onAnswer }: { offer: ChatOffer; onAnswer: (yes: boolean) => void }) {
  return (
    <div className="flex flex-wrap items-center gap-2 mt-2 pt-2 border-t border-white/5 text-[13px]" style={{ color: 'var(--text-dim)' }}>
      <span className="flex-1 min-w-[12rem]">{offer.line}</span>
      <button
        type="button"
        onClick={() => onAnswer(true)}
        className="h-8 px-3 rounded-full border border-white/15 hover:bg-white/10 transition-colors"
        style={{ color: 'var(--accent-blue)' }}
      >
        {t('chat_offer_yes')}
      </button>
      <button type="button" onClick={() => onAnswer(false)} className="h-8 px-3 rounded-full hover:bg-white/5 transition-colors">
        {t('chat_offer_not_now')}
      </button>
    </div>
  )
}

/** Marks a question an outside MCP client asked HQ, so it is not mistaken for something typed here. */
function McpLabel({ caller }: { caller: string }) {
  return (
    <div
      className="flex items-center gap-1.5 mb-1.5 text-[11px] min-w-0"
      style={{ color: 'var(--text-dim)' }}
      title={t('chat_via_mcp_title')}
    >
      <Plug className="w-3 h-3 shrink-0" />
      <span className="truncate">{t('chat_via_mcp', { caller })}</span>
    </div>
  )
}

/** Marks a message HQ posted for a watched session rather than in answer to the user. */
function DriverLabel({ driver }: { driver: DriverMeta }) {
  const drove = driver.mode === 'drive'
  return (
    <div
      className="flex items-center gap-1.5 mb-1.5 text-[11px] min-w-0"
      style={{ color: 'var(--text-dim)' }}
      title={drove ? t('chat_helper_acted') : t('chat_helper_update')}
    >
      <Terminal className="w-3 h-3 shrink-0" />
      <span className="truncate">
        {t('chat_helper_label', { id: driver.sessionId })}
        {driver.reason && ` · ${driver.reason}`}
      </span>
      {drove && <span className="shrink-0" style={{ color: 'var(--accent-amber)' }}>{t('chat_acted_for_you')}</span>}
      {!drove && (
        <Link to="/helpers/$id" params={{ id: driver.sessionId }} className="shrink-0 underline">
          {t('chat_answer_in_helpers')}
        </Link>
      )}
    </div>
  )
}
