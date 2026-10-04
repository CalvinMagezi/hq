import { create } from 'zustand'
import { MAX_ATTACHMENTS, isPreviewableImage, rejectReason, uploadChatFile, type ChatAttachment } from '~/lib/chatUploads'
import type { PendingAttachment } from './AttachmentChips'

// A chat that does not exist yet (the first message creates it) keeps its draft here.
export const NEW_CHAT_KEY = 'new'
const TEXT_STORAGE_KEY = 'hq.chat.drafts'

interface Draft {
  text: string
  attachments: PendingAttachment[]
}

const EMPTY: Draft = { text: '', attachments: [] }

function readSavedText(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(TEXT_STORAGE_KEY) ?? '{}') as Record<string, string>
  } catch {
    return {}
  }
}

/** Only the text survives a reload; picked files are File objects and live in memory. */
function saveText(drafts: Record<string, Draft>) {
  const text: Record<string, string> = {}
  for (const [key, d] of Object.entries(drafts)) if (d.text.trim()) text[key] = d.text
  try {
    localStorage.setItem(TEXT_STORAGE_KEY, JSON.stringify(text))
  } catch {
    // Storage can be blocked; drafts still work for this session.
  }
}

interface DraftState {
  drafts: Record<string, Draft>
  setText: (key: string, text: string) => void
  patchAttachment: (key: string, id: string, update: Partial<PendingAttachment>) => void
  addAttachments: (key: string, items: PendingAttachment[]) => void
  removeAttachment: (key: string, id: string) => void
  clear: (key: string) => void
}

const useDraftStore = create<DraftState>()((set, get) => {
  const update = (key: string, fn: (d: Draft) => Draft) => {
    const drafts = { ...get().drafts, [key]: fn(get().drafts[key] ?? EMPTY) }
    set({ drafts })
    return drafts
  }
  const initial: Record<string, Draft> = {}
  if (typeof window !== 'undefined') {
    for (const [key, text] of Object.entries(readSavedText())) initial[key] = { text, attachments: [] }
  }
  return {
    drafts: initial,
    setText: (key, text) => saveText(update(key, (d) => ({ ...d, text }))),
    patchAttachment: (key, id, patch) =>
      update(key, (d) => ({ ...d, attachments: d.attachments.map((a) => (a.id === id ? { ...a, ...patch } : a)) })),
    addAttachments: (key, items) => update(key, (d) => ({ ...d, attachments: [...d.attachments, ...items] })),
    removeAttachment: (key, id) =>
      update(key, (d) => {
        const gone = d.attachments.find((a) => a.id === id)
        if (gone?.previewUrl) URL.revokeObjectURL(gone.previewUrl)
        return { ...d, attachments: d.attachments.filter((a) => a.id !== id) }
      }),
    clear: (key) => {
      for (const a of get().drafts[key]?.attachments ?? []) if (a.previewUrl) URL.revokeObjectURL(a.previewUrl)
      saveText(update(key, () => EMPTY))
    },
  }
})

/**
 * The composer's text and files for one chat. Each chat keeps its own, so a
 * file picked in one chat can never be sent into another, and an upload that
 * finishes after switching chats lands in the chat it was picked in.
 */
export function useDraft(threadId: string | null | undefined) {
  const key = threadId ?? NEW_CHAT_KEY
  const draft = useDraftStore((s) => s.drafts[key]) ?? EMPTY
  const store = useDraftStore.getState

  const upload = async (item: PendingAttachment) => {
    try {
      store().patchAttachment(key, item.id, { status: 'ready', uploaded: await uploadChatFile(item.file), error: undefined })
    } catch (err) {
      store().patchAttachment(key, item.id, {
        status: 'error',
        error: err instanceof Error ? err.message : 'upload failed',
        retryable: true,
      })
    }
  }

  const addFiles = (files: File[]) => {
    const room = Math.max(0, MAX_ATTACHMENTS - draft.attachments.length)
    const added = files.map((file, i): PendingAttachment => {
      const reason = i >= room ? `limit is ${MAX_ATTACHMENTS} files` : rejectReason(file)
      return {
        id: `${Date.now()}-${i}-${file.name}`,
        file,
        status: reason ? 'error' : 'uploading',
        error: reason ?? undefined,
        previewUrl: !reason && isPreviewableImage(file.type) ? URL.createObjectURL(file) : undefined,
      }
    })
    store().addAttachments(key, added)
    for (const item of added) if (item.status === 'uploading') void upload(item)
  }

  const retryAttachment = (id: string) => {
    const item = draft.attachments.find((a) => a.id === id)
    if (!item?.retryable) return
    store().patchAttachment(key, id, { status: 'uploading', error: undefined, retryable: false })
    void upload(item)
  }

  /** Hand over the uploaded files and clear this chat's draft; failed ones go with it. */
  const takeReady = (): ChatAttachment[] => {
    const ready = draft.attachments.flatMap((a) => (a.status === 'ready' && a.uploaded ? [a.uploaded] : []))
    store().clear(key)
    return ready
  }

  return {
    text: draft.text,
    setText: (text: string) => store().setText(key, text),
    attachments: draft.attachments,
    addFiles,
    removeAttachment: (id: string) => store().removeAttachment(key, id),
    retryAttachment,
    takeReady,
  }
}

/** Puts refused text back into a chat's composer, unless something new was typed there since. */
export function restoreDraft(threadId: string, text: string) {
  const state = useDraftStore.getState()
  if (!state.drafts[threadId]?.text.trim()) state.setText(threadId, text)
}
