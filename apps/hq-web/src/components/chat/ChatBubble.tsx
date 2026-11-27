import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { Mic, Copy, Check, FilePlus, ListChecks, Pencil, Plug, RotateCw, Terminal } from 'lucide-react'
import type { ToolStep } from '~/store/threadStore'
import type { DriverMeta } from '~/lib/sessionsApi'
import type { ChatAttachment } from '~/lib/chatUploads'
import { deliverablesFirst, type ReplyFile } from '~/lib/replyFiles'
import { CREDITS_TOOLTIP, formatCredits, totalCredits, type StepCredit } from '~/lib/stepCredits'
import { ToolCallBlock } from './ToolCallBlock'
import { LiveLine } from './LiveLine'
import { FileCard, MadeCard, SourceChips } from './ReplyExtras'
import { RememberedChip } from './RememberedChip'
import { madeNotes, relevantSources, type MadeNote, type ReplySource } from '~/lib/replySources'
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
  /** Files this reply made, checked by the server. */
  files?: ReplyFile[]
  /** The notes HQ remembered and showed the model for this reply. */
  recalled?: number[]
}

interface Props {
  message: DisplayMessage
  /** Still streaming: panels show progress and the footer waits for the end. */
  live?: boolean
  /** Offered on the user's own saved messages. */
  onEdit?: () => void
  /** Offered on the last reply. */
  onRegenerate?: () => void
  /** Offered on HQ's saved replies: keep this one as a note in the Vault. */
  onSave?: () => void
  /** Offered on HQ's saved replies: start a task from this one. */
  onMakeTask?: () => void
  /** Live only: when the turn started, for the seconds on the live line. */
  startedAt?: number
  /** Live only: the live line offers it after ten seconds. */
  onStop?: () => void
  /** A source chip was pressed. */
  onOpenSource?: (source: ReplySource) => void
  /** A made note's Open was pressed. */
  onOpenMade?: (note: MadeNote) => void
  /** Helpers still working in real state. */
  runningHelpers?: number
}

/**
 * One chat message. A streaming reply renders through this same layout
 * (thinking, tools, then text), so nothing moves when it becomes a saved message.
 */
export function ChatBubble({ message, live = false, startedAt, onEdit, onRegenerate, onSave, onMakeTask, onStop, onOpenSource, onOpenMade, runningHelpers }: Props) {
  const [copied, setCopied] = useState(false)
  const isUser = message.role === 'user'
  const steps = message.toolSteps ?? []
  const replyCredits = isUser ? null : totalCredits(message.stepCredits)
  // What the reply read and made is only known once it is saved; while it streams, the live line speaks.
  const sources = !isUser && !live ? relevantSources(steps, message.content) : []
  const made = !isUser && !live ? madeNotes(steps) : []

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
      {!isUser && live && <LiveLine steps={steps} startedAt={startedAt} writing={message.content !== ''} thinking={!!message.reasoning} runningHelpers={runningHelpers} onStop={onStop} />}
      {message.attachments && <MessageAttachments attachments={message.attachments} />}

      {message.content && (
        isUser
          ? <p className="hq-msg-text">{message.content}</p>
          : <MarkdownViewer content={message.content} bare onOpenFile={(path) => onOpenMade?.({ title: path.split('/').pop() ?? path, path })} />
      )}
      {!isUser && !live && steps.length > 0 && <ToolCallBlock steps={steps} credits={message.stepCredits} />}
      {!isUser && !live && deliverablesFirst(message.files ?? []).map((f) => (
        <FileCard key={f.path} file={f} onOpenNote={(note) => onOpenMade?.(note)} />
      ))}
      {made.map((n) => (
        <MadeCard key={n.path} note={n} onOpen={(note) => onOpenMade?.(note)} />
      ))}
      {sources.length > 0 && <SourceChips sources={sources} onOpen={(s) => onOpenSource?.(s)} />}
      {!isUser && !live && message.recalled && <RememberedChip ids={message.recalled} />}

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
            {onSave && message.content && (
              <button type="button" onClick={onSave} className="a-ib sm" title={t('chat_save_vault')} aria-label={t('chat_save_vault')}>
                <FilePlus className="a-ic xs" aria-hidden="true" />
              </button>
            )}
            {onMakeTask && message.content && (
              <button type="button" onClick={onMakeTask} className="a-ib sm" title={t('chat_make_task')} aria-label={t('chat_make_task')}>
                <ListChecks className="a-ic xs" aria-hidden="true" />
              </button>
            )}
          </span>
        </div>
      )}
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
