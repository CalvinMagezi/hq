import { useEffect, useState } from 'react'
import { Link } from '@tanstack/react-router'
import { AlertCircle, FileText, Loader2, RotateCw, X } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { formatBytes, isPreviewableImage, vaultFileUrl, type ChatAttachment } from '~/lib/chatUploads'

/** A file picked in the composer, from pick to upload result. */
export interface PendingAttachment {
  id: string
  file: File
  status: 'uploading' | 'ready' | 'error'
  error?: string
  /** The upload itself failed (not a size or count limit), so trying again can work. */
  retryable?: boolean
  uploaded?: ChatAttachment
  /** Object URL for an image preview; revoked when the chip goes away. */
  previewUrl?: string
}

const THUMB = 'w-10 h-10 rounded-md object-cover shrink-0 bg-white/5'

/** The row of files waiting to be sent, above the message box. Scrolls sideways on a phone. */
export function ComposerAttachments({
  items,
  onRemove,
  onRetry,
}: {
  items: PendingAttachment[]
  onRemove: (id: string) => void
  onRetry: (id: string) => void
}) {
  if (items.length === 0) return null
  return (
    <div className="flex gap-2 overflow-x-auto pb-2 max-w-4xl mx-auto overscroll-contain">
      {items.map((item) => (
        <div
          key={item.id}
          className={`relative flex items-center gap-2 pl-1.5 pr-9 py-1.5 rounded-xl border shrink-0 max-w-[14rem] ${
            item.status === 'error' ? 'border-red-500/40 bg-red-500/5' : 'border-white/10 bg-white/5'
          }`}
          title={item.error ?? item.file.name}
        >
          {item.previewUrl ? (
            <img src={item.previewUrl} alt="" className={THUMB} />
          ) : (
            <div className={`${THUMB} flex items-center justify-center`}>
              <FileText className="w-4 h-4 text-neutral-400" />
            </div>
          )}
          <div className="min-w-0 text-[11px] leading-tight">
            <div className="truncate text-neutral-200">{item.file.name}</div>
            <div className="flex items-center gap-1 text-neutral-500">
              {item.status === 'uploading' && <Loader2 className="w-3 h-3 animate-spin" />}
              {item.status === 'error' && <AlertCircle className="w-3 h-3 text-red-400" />}
              <span className="truncate">{item.status === 'error' ? item.error : formatBytes(item.file.size)}</span>
            </div>
            {item.retryable && (
              <button
                type="button"
                onClick={() => onRetry(item.id)}
                className="flex items-center gap-1 mt-0.5 text-emerald-400 hover:underline"
              >
                <RotateCw className="w-3 h-3" /> Retry
              </button>
            )}
          </div>
          <button
            type="button"
            onClick={() => onRemove(item.id)}
            className="absolute right-0.5 top-1/2 -translate-y-1/2 w-8 h-8 flex items-center justify-center rounded-full text-neutral-400 hover:text-white hover:bg-white/10"
            title={`Remove ${item.file.name}`}
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      ))}
    </div>
  )
}

/** Files a sent message carried. Each opens in the vault viewer. */
export function MessageAttachments({ attachments }: { attachments: ChatAttachment[] }) {
  const closeOverlay = useHQStore((s) => s.setGlobalChatOpen)
  if (attachments.length === 0) return null
  return (
    <div className="flex flex-wrap gap-2 mb-2">
      {attachments.map((a) => (
        <Link
          key={a.path}
          to="/vault/$"
          params={{ _splat: a.path }}
          onClick={() => closeOverlay(false)}
          className="flex items-center gap-2 pl-1.5 pr-3 py-1.5 rounded-xl border border-white/10 bg-black/20 hover:bg-white/5 max-w-full min-w-0"
          title={`Open ${a.name}`}
        >
          {isPreviewableImage(a.mime) ? (
            <VaultThumb path={a.path} />
          ) : (
            <div className={`${THUMB} flex items-center justify-center`}>
              <FileText className="w-4 h-4 text-neutral-400" />
            </div>
          )}
          <div className="min-w-0 text-[11px] leading-tight">
            <div className="truncate text-neutral-200">{a.name}</div>
            <div className="text-neutral-500">{formatBytes(a.size)}</div>
          </div>
        </Link>
      ))}
    </div>
  )
}

function VaultThumb({ path }: { path: string }) {
  const [url, setUrl] = useState<string | null>(null)
  useEffect(() => {
    let created: string | null = null
    let cancelled = false
    vaultFileUrl(path)
      .then((u) => {
        if (cancelled) URL.revokeObjectURL(u)
        else setUrl((created = u))
      })
      .catch(() => {})
    return () => {
      cancelled = true
      if (created) URL.revokeObjectURL(created)
    }
  }, [path])
  return url ? <img src={url} alt="" className={THUMB} loading="lazy" /> : <div className={`${THUMB} animate-pulse`} />
}
