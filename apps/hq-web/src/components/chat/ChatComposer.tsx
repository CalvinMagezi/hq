import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from 'react'
import { t } from '~/strings'
import { Plus, Send, Square, WifiOff } from 'lucide-react'
import { SlashCommandPalette, SLASH_COMMANDS, matchingSlashCommands, isSlashInput } from './SlashCommandPalette'
import { enterSendsMessage, readEnterSends, writeEnterSends } from '~/lib/enterRule'
import { tooBig } from '~/lib/chatUploads'
import { ComposerAttachments, type PendingAttachment } from './AttachmentChips'

// The box grows with its text up to this height, then scrolls.
const MAX_INPUT_HEIGHT_PX = 200
// Phones have no Shift key for a newline, so there Enter adds one and only the button sends.
const TOUCH_QUERY = '(pointer: coarse)'
interface Props {
  value: string
  onChange: (value: string) => void
  /** Sends the given text, or the current input when called with none. */
  onSend: (content?: string) => void
  onStop: () => void
  /** A reply is streaming, so the button stops it instead of sending. */
  running: boolean
  attachments: PendingAttachment[]
  onAddFiles: (files: File[]) => void
  onRemoveAttachment: (id: string) => void
  onRetryAttachment: (id: string) => void
  /** The socket is up; while it is down the draft is kept and sending waits. */
  connected: boolean
  /** With the socket down, a plain message can still be sent: it waits in the outbox. */
  canQueue?: boolean
  /** An edited message keeps its earlier files, so it can be sent with no new text or files. */
  hasKeptFiles?: boolean
}

/** Message input with attachments, the slash-command palette and the send/stop button. */
export const ChatComposer = forwardRef<HTMLTextAreaElement, Props>(function ChatComposer(
  { value, onChange, onSend, onStop, running, attachments, onAddFiles, onRemoveAttachment, onRetryAttachment, connected, canQueue = false, hasKeptFiles = false },
  ref,
) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [enterSends, setEnterSends] = useState<boolean | null>(() => (typeof window === 'undefined' ? null : readEnterSends()))
  useImperativeHandle(ref, () => textareaRef.current as HTMLTextAreaElement)

  useLayoutEffect(() => {
    const el = textareaRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, MAX_INPUT_HEIGHT_PX)}px`
  }, [value])

  const uploading = attachments.some((a) => a.status === 'uploading')
  const hasReady = attachments.some((a) => a.status === 'ready')
  // One reply at a time per chat: while one runs the button stops it, and Enter waits.
  const queueing = !connected && canQueue && attachments.length === 0 && !hasKeptFiles
  const hasBigFile = attachments.some((a) => tooBig(a.file))
  const canSend = (connected || queueing) && !running && !uploading && !hasBigFile && (!!value.trim() || hasReady || hasKeptFiles)

  const pickCommand = (cmd: string) => {
    const def = SLASH_COMMANDS.find((c) => c.name === cmd)
    if (def && !def.args) {
      onChange('')
      onSend(cmd)
    } else {
      onChange(cmd + ' ')
    }
  }

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Escape' && running) {
      onStop()
      return
    }
    const paletteActive = isSlashInput(value) && !value.includes(' ') && matchingSlashCommands(value).length > 0
    const send = enterSendsMessage({
      key: e.key,
      shiftKey: e.shiftKey,
      isComposing: e.nativeEvent.isComposing,
      keyCode: e.keyCode,
      coarse: window.matchMedia(TOUCH_QUERY).matches,
      paletteActive,
      enterSends,
    })
    if (!send) return
    e.preventDefault()
    if (canSend) onSend()
  }

  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(e.clipboardData.files)
    if (files.length === 0) return
    e.preventDefault()
    onAddFiles(files)
  }

  const sendTitle = !connected ? t('chat_banner_reconnecting') : uploading ? t('chat_send_waiting_files') : t('chat_send')

  return (
    <div className="hq-comp">
      {isSlashInput(value) && <SlashCommandPalette input={value} onSelect={pickCommand} />}

      {!connected && !canQueue && (
        <p role="status" className="a-cap a-warn-t hq-comp-note">
          <WifiOff className="a-ic xs" aria-hidden="true" /> {t('outbox_shared_send')}
        </p>
      )}
      {hasBigFile && (
        <p role="status" className="a-cap a-warn-t hq-comp-note">
          {t('chat_remove_big')}
        </p>
      )}
      <ComposerAttachments items={attachments} onRemove={onRemoveAttachment} onRetry={onRetryAttachment} />

      <div className="a-comp">
      <div className="a-comp-in">
        <input
          ref={fileInputRef}
          type="file"
          multiple
          hidden
          onChange={(e) => {
            const files = Array.from(e.target.files ?? [])
            // Clearing lets the same file be picked again after it was removed.
            e.target.value = ''
            if (files.length > 0) onAddFiles(files)
          }}
        />
        <button type="button" className="a-ib" onClick={() => fileInputRef.current?.click()} aria-label={t('chat_attach')}>
          <Plus className="a-ic" aria-hidden="true" />
        </button>
        <div className="a-comp-field">
          <label className="a-sr" htmlFor="hq-composer">
            {t('chat_message_hq')}
          </label>
          <textarea
            id="hq-composer"
            ref={textareaRef}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={onKeyDown}
            onPaste={onPaste}
            placeholder={t('chat_message_hq')}
            rows={1}
            enterKeyHint="enter"
          />
        </div>
        {running ? (
          <button type="button" className="a-send stop" onClick={onStop} aria-label={t('chat_stop')}>
            <Square className="a-ic sm" aria-hidden="true" />
          </button>
        ) : (
          <button type="button" className="a-send" onClick={() => onSend()} disabled={!canSend} title={sendTitle} aria-label={t('chat_send')}>
            <Send className="a-ic sm" aria-hidden="true" />
          </button>
        )}
      </div>
      </div>
      <label className="a-cap a-muted hq-comp-enter" title={t('chat_enter_sends_hint')}>
        <input
          type="checkbox"
          checked={enterSends ?? true}
          onChange={(e) => {
            writeEnterSends(e.target.checked)
            setEnterSends(e.target.checked)
          }}
        />
        {t('chat_enter_sends')}
      </label>
    </div>
  )
})
