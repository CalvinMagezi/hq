import { t } from '~/strings'
import { createNote, deleteNote } from './vaultApi'
import { replyTitle } from './replyText'

/** Where a note saved from a chat goes: the folder new notes go to unless the person picks another. */
export const REPLY_FOLDER = 'Notebooks/Inbox'

export interface SavedReply {
  path: string
  /** Puts the note in the Trash again. */
  undo: () => Promise<unknown>
}

interface Deps {
  create: typeof createNote
  remove: typeof deleteNote
}

/**
 * Saves a reply as a note in the Vault. Its title is the reply's first heading or sentence. A second press
 * on the same reply lands on the same note, because the message id is the create's client id.
 */
export async function saveReplyToVault(content: string, messageId: string, deps: Deps = { create: createNote, remove: deleteNote }): Promise<SavedReply> {
  const title = replyTitle(content) || t('chat_save_default_title')
  const { path } = await deps.create(title, content, REPLY_FOLDER, `chat-${messageId}`)
  return { path, undo: () => deps.remove(path) }
}

/** What goes in the composer for "Make a task": a request the person can read, change and send. */
export function makeTaskDraft(content: string): string {
  return t('chat_make_task_draft', { title: replyTitle(content) || t('chat_save_default_title') })
}
