import { hqJson } from './hqAuth'

/** A folder the owner allowed for one chat: its commands and edits there run without asking. */
export interface ChatFolder {
  id: number
  root: string
  /** Unix seconds. */
  expires_at: number
  uses: number
}

export const chatFoldersKey = (threadId: string | null) => ['chat-folders', threadId]

export async function fetchChatFolders(threadId: string): Promise<ChatFolder[]> {
  const res = await hqJson<{ folders: ChatFolder[] }>(`/api/chat-folders/${encodeURIComponent(threadId)}`)
  return res.folders
}

export async function revokeChatFolder(threadId: string, id: number): Promise<void> {
  await hqJson(`/api/chat-folders/${encodeURIComponent(threadId)}/${id}`, 'DELETE')
}
