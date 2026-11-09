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

export type OfferAnswer = 'yes' | 'not_now' | 'scheduled'

const LATER_TODAY_MS = 3 * 60 * 60 * 1000
const MORNING_HOUR = 9
const NEXT_WEEK_DAYS = 7
/** The server schedules only next steps, which make sense on their own later. */
const SCHEDULABLE_PREFIX = 'next_'

export const canSchedule = (offer: ChatOffer): boolean => offer.kind.startsWith(SCHEDULABLE_PREFIX)

/** What Schedule offers, each as unix seconds. */
export function scheduleChoices(now: Date) {
  const morning = (days: number) => {
    const d = new Date(now)
    d.setDate(d.getDate() + days)
    d.setHours(MORNING_HOUR, 0, 0, 0)
    return Math.floor(d.getTime() / 1000)
  }
  return [
    { key: 'chat_offer_later_today', at: Math.floor((now.getTime() + LATER_TODAY_MS) / 1000) },
    { key: 'chat_offer_tomorrow', at: morning(1) },
    { key: 'chat_offer_next_week', at: morning(NEXT_WEEK_DAYS) },
  ] as const
}

/** Records the answer on this HQ and hides the line at once; the saved reply loses it too. */
export async function answerOffer(threadId: string, messageId: string, answer: OfferAnswer, at?: number): Promise<void> {
  useThreadStore.setState((s) => ({
    threadMessages: {
      ...s.threadMessages,
      [threadId]: (s.threadMessages[threadId] ?? []).map((m) => (m.messageId === messageId ? { ...m, offer: undefined } : m)),
    },
  }))
  await hqJson('/api/offers/answer', 'POST', { message_id: messageId, answer, at })
}
