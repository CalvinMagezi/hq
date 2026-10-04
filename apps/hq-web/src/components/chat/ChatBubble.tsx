import { useState } from 'react'
import { Copy, Check, Pencil, Plug, RotateCw, Terminal, User } from 'lucide-react'
import type { ToolStep } from '~/store/threadStore'
import type { DriverMeta } from '~/lib/sessionsApi'
import type { ChatAttachment } from '~/lib/chatUploads'
import { CREDITS_TOOLTIP, formatCredits, totalCredits, type StepCredit } from '~/lib/stepCredits'
import { ToolCallBlock } from './ToolCallBlock'
import { ThinkingPanel } from './ThinkingPanel'
import { MessageAttachments } from './AttachmentChips'
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
}

interface Props {
  message: DisplayMessage
  /** Still streaming: panels show progress and the footer waits for the end. */
  live?: boolean
  /** Offered on the user's own saved messages. */
  onEdit?: () => void
  /** Offered on the last reply. */
  onRegenerate?: () => void
}

// Hover reveals actions on a desktop; a touch screen has no hover, so they always show there.
const ACTION_BUTTON =
  'p-1.5 -m-1 rounded hover:bg-white/10 transition-colors opacity-0 group-hover:opacity-100 [@media(hover:none)]:opacity-100 flex items-center gap-1'

/**
 * One chat message. A streaming reply renders through this same layout
 * (thinking, tools, then text), so nothing moves when it becomes a saved message.
 */
export function ChatBubble({ message, live = false, onEdit, onRegenerate }: Props) {
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
                background: 'rgba(0, 255, 163, 0.08)',
                border: '1px solid rgba(0, 255, 163, 0.2)',
                borderRadius: '16px 4px 16px 16px',
                color: 'var(--text-primary)',
              }
            : {
                background: 'rgba(255, 255, 255, 0.03)',
                border: '1px solid rgba(255, 255, 255, 0.08)',
                borderRadius: '4px 16px 16px 16px',
                color: 'var(--text-primary)',
              }
        }
      >
        {isUser && message.viaMcp && <McpLabel caller={message.viaMcp} />}
        {!isUser && message.driver && <DriverLabel driver={message.driver} />}
        {!isUser && message.reasoning && <ThinkingPanel content={message.reasoning} live={live} />}
        {!isUser && steps.length > 0 && <ToolCallBlock steps={steps} credits={message.stepCredits} live={live} />}
        {message.attachments && <MessageAttachments attachments={message.attachments} />}

        {isUser ? (
          message.content && <p className="whitespace-pre-wrap break-words">{message.content}</p>
        ) : (
          message.content && <MarkdownViewer content={message.content} bare />
        )}
        {waiting && <div className="text-xs text-neutral-500 animate-pulse">HQ is working...</div>}

        {!live && (
          <div className="flex items-center justify-between mt-1.5 pt-1 border-t border-white/5 opacity-60 text-xs font-mono" style={{ color: 'var(--text-dim)' }}>
            <span>
              {new Date(message.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
              {message.stopped && <span className="ml-2" style={{ color: 'var(--accent-amber)' }}>stopped</span>}
              {replyCredits !== null && (
                <span className="ml-2" title={CREDITS_TOOLTIP}>{formatCredits(replyCredits)}</span>
              )}
            </span>
            <span className="flex items-center gap-3">
              {onEdit && (
                <button type="button" onClick={onEdit} className={ACTION_BUTTON} title="Edit and resend">
                  <Pencil className="w-3.5 h-3.5" />
                </button>
              )}
              {onRegenerate && (
                <button type="button" onClick={onRegenerate} className={ACTION_BUTTON} title="Regenerate this reply">
                  <RotateCw className="w-3.5 h-3.5" />
                </button>
              )}
              {message.content && (
                <button type="button" onClick={handleCopy} className={ACTION_BUTTON} title="Copy message">
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

/** Marks a question an outside MCP client asked HQ, so it is not mistaken for something typed here. */
function McpLabel({ caller }: { caller: string }) {
  return (
    <div
      className="flex items-center gap-1.5 mb-1.5 text-[11px] font-mono min-w-0"
      style={{ color: 'var(--text-dim)' }}
      title="Asked through HQ's MCP endpoint by an outside client"
    >
      <Plug className="w-3 h-3 shrink-0" />
      <span className="truncate">via MCP: {caller}</span>
    </div>
  )
}

/** Marks a message HQ posted for a watched session rather than in answer to the user. */
function DriverLabel({ driver }: { driver: DriverMeta }) {
  const drove = driver.mode === 'drive'
  return (
    <div
      className="flex items-center gap-1.5 mb-1.5 text-[11px] font-mono min-w-0"
      style={{ color: 'var(--text-dim)' }}
      title={drove ? 'HQ acted in this session on your behalf' : 'An update from a session this chat watches'}
    >
      <Terminal className="w-3 h-3 shrink-0" />
      <span className="truncate">
        Session {driver.sessionId}
        {driver.reason && ` · ${driver.reason}`}
      </span>
      {drove && <span className="shrink-0" style={{ color: 'var(--accent-amber)' }}>HQ acted for you</span>}
    </div>
  )
}
