import { hqJson } from './hqAuth'
import { useThreadStore, type ChatOffer } from '~/store/threadStore'

/** How the server saves an offer under a reply (`meta.offer`), chosen on this device by HQ. */
export interface ApiOffer {
  kind: string
  line: string
  reply: string
  send: boolean
}

export function toOffer(o: ApiOffer | null | undefined): ChatOffer | undefined {
  if (!o?.kind || !o.line || !o.reply) return undefined
  return { kind: o.kind, line: o.line, reply: o.reply, send: o.send === true }
}

export type OfferAnswer = 'yes' | 'not_now'

/** Records the answer on this HQ and hides the line at once; the saved reply loses it too. */
export async function answerOffer(threadId: string, messageId: string, answer: OfferAnswer): Promise<void> {
  useThreadStore.setState((s) => ({
    threadMessages: {
      ...s.threadMessages,
      [threadId]: (s.threadMessages[threadId] ?? []).map((m) => (m.messageId === messageId ? { ...m, offer: undefined } : m)),
    },
  }))
  await hqJson('/api/offers/answer', 'POST', { message_id: messageId, answer })
}
