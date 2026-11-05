// Chat messages written while HQ is out of reach wait in the outbox ("Waiting to send") and go out one
// at a time once the socket is back (docs/plans/ux-run/web/wave-0/02-shared-parts.md rules 11 to 15).
// HQ drops a repeated client id, so a message whose answer was lost is never sent twice.
import { useCallback, useEffect, useRef, useState } from 'react'
import { useWS } from '~/context/WebSocketContext'
import { type ConnectionState, useSharedConnection } from '~/lib/connection'
import { count } from '~/lib/noticed'
import { browserStore, createOutbox, newClientId, type OutboxEntry, type OutboxLineState, type SendResult } from '~/lib/outbox'
import { currentHqId } from '~/lib/savedCopy'
import { isSharedComputer } from '~/lib/sharedComputer'

const outbox = createOutbox(browserStore)
/** How long a sent message waits for HQ to say it has it before the outbox tries again. */
const ACK_TIMEOUT_MS = 15_000
const NOTICED_VERSION = 'web'

export interface ChatPayload {
  content: string
}

/** The outbox's line for a connection state; Connected and the rest show "It is next." */
export function lineState(state: ConnectionState): OutboxLineState {
  if (state === 'starting' || state === 'unreachable' || state === 'offline') return state
  return 'next'
}

export function useChatOutbox(threadId: string | null) {
  const { send, subscribe, connected } = useWS()
  const state = useSharedConnection()
  const [entries, setEntries] = useState<OutboxEntry[]>([])
  const [sending, setSending] = useState<{ n: number; m: number; client_id: string } | undefined>()
  const draining = useRef(false)

  const reload = useCallback(async () => {
    const id = currentHqId()
    setEntries(id ? await outbox.list(id) : [])
  }, [])

  useEffect(() => {
    void reload()
  }, [reload])

  /** Sends one message and waits for HQ's answer about it, by its client id. */
  const sendOne = useCallback(
    (entry: OutboxEntry): Promise<SendResult> =>
      new Promise((resolve) => {
        const payload = entry.payload as ChatPayload
        let done = false
        const finish = (r: SendResult) => {
          if (done) return
          done = true
          stop()
          clearTimeout(timer)
          resolve(r)
        }
        const stop = subscribe((msg) => {
          if (msg.client_id !== entry.client_id) return
          if (msg.type === 'turn_start') finish('sent')
          if (msg.type === 'chat_duplicate') {
            count('outbox.repeats_dropped', NOTICED_VERSION)
            finish('sent')
          }
          // A reply already running in that chat: wait for it, then try again.
          if (msg.type === 'chat_rejected') finish(msg.running === true ? 'retry' : 'gone')
        })
        const timer = setTimeout(() => finish('retry'), ACK_TIMEOUT_MS)
        const left = send({ type: 'chat', content: payload.content, thread_id: entry.thread_id, client_id: entry.client_id })
        if (!left) finish('retry')
      }),
    [send, subscribe],
  )

  const drain = useCallback(async () => {
    const id = currentHqId()
    if (!id || draining.current || !connected) return
    draining.current = true
    // The drain reports "n of m" just before it sends each entry.
    let at = { n: 0, m: 0 }
    try {
      await outbox.drain(
        id,
        (entry) => {
          setSending({ ...at, client_id: entry.client_id })
          return sendOne(entry)
        },
        (n, m) => {
          at = { n, m }
        },
      )
    } finally {
      draining.current = false
      setSending(undefined)
      await reload()
    }
  }, [connected, reload, sendOne])

  // Back online, or a reply ended (a chat that was busy can take the next one).
  useEffect(() => {
    if (connected) void drain()
  }, [connected, drain])
  useEffect(
    () =>
      subscribe((msg) => {
        if (msg.type === 'turn_end') void drain()
      }),
    [subscribe, drain],
  )

  /** Puts a message in the outbox; the refusal sentence when 100 already wait. */
  const queue = useCallback(
    async (tid: string, content: string): Promise<string | null> => {
      const id = currentHqId()
      if (!id) return null
      const added = await outbox.add({
        client_id: `local-${newClientId()}`,
        kind: 'message',
        hq_id: id,
        thread_id: tid,
        payload: { content } satisfies ChatPayload,
        created_at: Date.now(),
        attempts: 0,
      })
      await reload()
      return added.ok ? null : added.message
    },
    [reload],
  )

  const remove = useCallback(async (clientId: string) => {
    const id = currentHqId()
    if (id) await outbox.remove(id, clientId)
    await reload()
  }, [reload])

  const sendNow = useCallback(async (clientId: string) => {
    const id = currentHqId()
    if (id) await outbox.sendNow(id, clientId)
    await reload()
    void drain()
  }, [reload, drain])

  return {
    entries: entries.filter((e) => e.thread_id === threadId),
    state: lineState(state),
    sending,
    queue,
    remove,
    sendNow,
    enabled: !isSharedComputer(),
  }
}
