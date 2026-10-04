import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from 'react'
import { Paperclip, Send, Square, WifiOff } from 'lucide-react'
import { SlashCommandPalette, SLASH_COMMANDS, matchingSlashCommands, isSlashInput } from './SlashCommandPalette'
import { ComposerAttachments, type PendingAttachment } from './AttachmentChips'

// The box grows with its text up to this height, then scrolls.
const MAX_INPUT_HEIGHT_PX = 200
// Phones have no Shift key for a newline, so there Enter adds one and only the button sends.
const TOUCH_QUERY = '(pointer: coarse)'
// The full hint wraps to two lines at phone width and doubles the box's height.
const WIDE_QUERY = '(min-width: 640px)'
const PLACEHOLDER_WIDE = 'Message HQ, / for commands'
const PLACEHOLDER_NARROW = 'Message HQ'

function usePlaceholder() {
  const [wide, setWide] = useState(() => typeof window !== 'undefined' && window.matchMedia(WIDE_QUERY).matches)
  useEffect(() => {
    const mq = window.matchMedia(WIDE_QUERY)
    const onChange = () => setWide(mq.matches)
    onChange()
    mq.addEventListener('change', onChange)
    return () => mq.removeEventListener('change', onChange)
  }, [])
  return wide ? PLACEHOLDER_WIDE : PLACEHOLDER_NARROW
}

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
  /** An edited message keeps its earlier files, so it can be sent with no new text or files. */
  hasKeptFiles?: boolean
}

/** Message input with attachments, the slash-command palette and the send/stop button. */
export const ChatComposer = forwardRef<HTMLTextAreaElement, Props>(function ChatComposer(
  { value, onChange, onSend, onStop, running, attachments, onAddFiles, onRemoveAttachment, onRetryAttachment, connected, hasKeptFiles = false },
  ref,
) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const placeholder = usePlaceholder()
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
  const canSend = connected && !running && !uploading && (!!value.trim() || hasReady || hasKeptFiles)

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
    if (e.key !== 'Enter' || e.shiftKey || e.nativeEvent.isComposing) return
    if (window.matchMedia(TOUCH_QUERY).matches) return
    const paletteActive = isSlashInput(value) && !value.includes(' ') && matchingSlashCommands(value).length > 0
    if (paletteActive) return
    e.preventDefault()
    if (canSend) onSend()
  }

  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(e.clipboardData.files)
    if (files.length === 0) return
    e.preventDefault()
    onAddFiles(files)
  }

  const sendTitle = !connected ? 'Reconnecting, your message will wait' : uploading ? 'Waiting for uploads' : 'Send'

  return (
    <div className="px-3 pt-2 pb-3 sm:px-4 sm:pb-4 border-t border-white/10 shrink-0 relative bg-neutral-950/80 backdrop-blur-md">
      {isSlashInput(value) && <SlashCommandPalette input={value} onSelect={pickCommand} />}

      {!connected && (
        <div className="flex items-center justify-center gap-1.5 pb-2 text-[11px] font-mono" style={{ color: 'var(--accent-amber)' }}>
          <WifiOff className="w-3 h-3" /> Reconnecting. Your draft is kept and can be sent once HQ is back.
        </div>
      )}

      <ComposerAttachments items={attachments} onRemove={onRemoveAttachment} onRetry={onRetryAttachment} />

      <div className="flex items-end gap-2 sm:gap-3 max-w-4xl mx-auto">
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => {
            const files = Array.from(e.target.files ?? [])
            // Clearing lets the same file be picked again after it was removed.
            e.target.value = ''
            if (files.length > 0) onAddFiles(files)
          }}
        />
        <button
          type="button"
          onClick={() => fileInputRef.current?.click()}
          className="w-10 h-10 rounded-full flex items-center justify-center text-neutral-400 hover:text-white hover:bg-white/10 shrink-0"
          title="Attach files"
        >
          <Paperclip className="w-4 h-4" />
        </button>

        <textarea
          ref={textareaRef}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          placeholder={placeholder}
          rows={1}
          enterKeyHint="enter"
          className="flex-1 min-w-0 rounded-2xl px-4 py-2.5 text-base sm:text-sm leading-6 outline-none resize-none bg-white/5 border border-white/10 text-white caret-emerald-400 focus:border-emerald-500/40 overflow-y-auto"
          style={{ maxHeight: MAX_INPUT_HEIGHT_PX }}
        />

        {running ? (
          <button
            type="button"
            onClick={onStop}
            className="w-10 h-10 rounded-full flex items-center justify-center bg-amber-500 text-black hover:brightness-110 shrink-0"
            title="Stop response"
          >
            <Square className="w-4 h-4 fill-current" />
          </button>
        ) : (
          <button
            type="button"
            onClick={() => onSend()}
            disabled={!canSend}
            className="w-10 h-10 rounded-full flex items-center justify-center bg-emerald-400 text-black font-bold disabled:opacity-30 hover:brightness-110 shrink-0"
            title={sendTitle}
          >
            <Send className="w-4 h-4" />
          </button>
        )}
      </div>
    </div>
  )
})
