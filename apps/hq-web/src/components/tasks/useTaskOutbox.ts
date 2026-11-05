// A task made while HQ is out of reach waits in the outbox and is created once, by client id, when it is back.
import { useCallback, useEffect, useRef, useState } from 'react'
import { useSharedConnection } from '~/lib/connection'
import { browserStore, createOutbox, newClientId, type OutboxEntry } from '~/lib/outbox'
import { currentHqId } from '~/lib/savedCopy'
import { isSharedComputer } from '~/lib/sharedComputer'
import { createTaskClient, type CreateTaskInput, type TaskItem } from '~/lib/tasksApi'
import { lineState } from '../chat/useChatOutbox'

const outbox = createOutbox(browserStore)

export function titleOf(entry: OutboxEntry): string {
  return (entry.payload as CreateTaskInput).title
}

export function useTaskOutbox(onCreated: (task: TaskItem) => void) {
  const state = useSharedConnection()
  const connected = state === 'connected'
  const [entries, setEntries] = useState<OutboxEntry[]>([])
  const draining = useRef(false)

  const reload = useCallback(async () => {
    const id = currentHqId()
    setEntries(id ? (await outbox.list(id)).filter((e) => e.kind === 'task') : [])
  }, [])

  const drain = useCallback(async () => {
    const id = currentHqId()
    if (!id || draining.current) return
    draining.current = true
    try {
      await outbox.drain(
        id,
        async (entry) => {
          try {
            onCreated(await createTaskClient({ ...(entry.payload as CreateTaskInput), client_id: entry.client_id }))
            return 'sent'
          } catch {
            return 'retry'
          }
        },
        undefined,
        'task',
      )
    } finally {
      draining.current = false
      await reload()
    }
  }, [onCreated, reload])

  useEffect(() => {
    void reload()
  }, [reload])
  useEffect(() => {
    if (connected) void drain()
  }, [connected, drain])

  /** Keeps a task for later; the refusal sentence when the outbox is full, null when kept or not allowed. */
  const queue = useCallback(
    async (input: CreateTaskInput): Promise<string | null> => {
      const id = currentHqId()
      if (!id || isSharedComputer()) return null
      const added = await outbox.add({
        client_id: input.client_id ?? newClientId(),
        kind: 'task',
        hq_id: id,
        payload: input,
        created_at: Date.now(),
        attempts: 0,
      })
      await reload()
      return added.ok ? null : added.message
    },
    [reload],
  )

  const remove = useCallback(
    async (clientId: string) => {
      const id = currentHqId()
      if (id) await outbox.remove(id, clientId)
      await reload()
    },
    [reload],
  )

  const sendNow = useCallback(() => drain(), [drain])

  return { entries, state: lineState(state), queue, remove, sendNow, enabled: !isSharedComputer() }
}
