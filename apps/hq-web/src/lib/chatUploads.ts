import { hqFetch, readJson } from './hqAuth'

/** A file saved in the vault for a chat message. `path` is vault-relative. */
export interface ChatAttachment {
  name: string
  path: string
  mime: string
  size: number
}

const BYTES_PER_MB = 1024 * 1024
// Mirrors the server's caps in crates/hq-web/src/chat_uploads.rs.
export const MAX_UPLOAD_BYTES = 25 * BYTES_PER_MB
export const MAX_ATTACHMENTS = 10

// The server appends this to a saved user message; an unclosed tag is a preview cut mid-marker.
const MARKER_RE = /\s*<hq-attachments>([\s\S]*?)(?:<\/hq-attachments>|$)/

const VISION_MIMES = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp'])

export function isPreviewableImage(mime: string) {
  return VISION_MIMES.has(mime)
}

export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < BYTES_PER_MB) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / BYTES_PER_MB).toFixed(1)} MB`
}

/** Why a file can't be attached, or null when it can. */
export function rejectReason(file: File): string | null {
  if (file.size === 0) return 'empty file'
  if (file.size > MAX_UPLOAD_BYTES) return `over ${MAX_UPLOAD_BYTES / BYTES_PER_MB} MB`
  return null
}

/** Save one file for the next message; the reply carries its vault path. */
export async function uploadChatFile(file: File): Promise<ChatAttachment> {
  const res = await hqFetch(`/api/chat/uploads?name=${encodeURIComponent(file.name)}`, {
    method: 'POST',
    headers: { 'Content-Type': file.type || 'application/octet-stream' },
    body: file,
  })
  return readJson<ChatAttachment>(res)
}

/** Split a saved message into its text and the files it carried. */
export function splitAttachments(content: string): { text: string; attachments: ChatAttachment[] } {
  const match = MARKER_RE.exec(content)
  if (!match) return { text: content, attachments: [] }
  const text = content.slice(0, match.index)
  try {
    const parsed = JSON.parse(match[1]) as ChatAttachment[]
    return { text, attachments: Array.isArray(parsed) ? parsed : [] }
  } catch {
    return { text, attachments: [] }
  }
}

/** A blob URL for a vault file, fetched with the auth header. The caller revokes it. */
export async function vaultFileUrl(path: string): Promise<string> {
  const res = await hqFetch(`/api/vault-asset?path=${encodeURIComponent(path)}`)
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  return URL.createObjectURL(await res.blob())
}
