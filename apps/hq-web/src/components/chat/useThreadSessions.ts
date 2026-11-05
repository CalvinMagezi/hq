import { useCallback, useEffect, useRef, useState } from 'react'
import { useWS } from '~/context/WebSocketContext'
import { visibleTimer } from '~/lib/visibleTimer'
import { useHQStore } from '~/store/hqStore'
import { t } from '~/strings'
import { sessionsApi, type WatchedSession } from '~/lib/sessionsApi'

// A fallback for sync events lost while the socket was down or the tab slept.
const POLL_MS = 60_000

const errorText = (err: unknown, fallback: string) => (err instanceof Error ? err.message : fallback)

/**
 * The coding-agent sessions a chat is watching, kept current by `sessions:sync`
 * events and, while `polling`, a slow refetch. Does nothing unless `active`.
 */
export function useThreadSessions(threadId: string | null, active: boolean, polling: boolean) {
  const { subscribe, connected } = useWS()
  // Keyed by thread so a slow response for the previous chat never shows under this one.
  const [loaded, setLoaded] = useState<{ threadId: string; sessions: WatchedSession[] } | null>(null)
  const [busy, setBusy] = useState<Set<string>>(new Set())
  const currentThread = useRef(threadId)
  currentThread.current = threadId

  const refresh = useCallback(async () => {
    const tid = currentThread.current
    if (!tid) return
    try {
      const sessions = await sessionsApi.listForThread(tid)
      if (currentThread.current === tid) setLoaded({ threadId: tid, sessions })
    } catch (err) {
      console.warn('useThreadSessions: failed to load sessions', err)
    }
  }, [])

  // A reconnect refetches too: sync events sent while the socket was down are gone.
  useEffect(() => {
    if (active && threadId) void refresh()
  }, [active, threadId, connected, refresh])

  useEffect(() => {
    if (!active || !threadId) return
    return subscribe((msg) => {
      if (msg.type === 'sessions:sync' && msg.thread_id === threadId) void refresh()
    })
  }, [active, threadId, subscribe, refresh])

  useEffect(() => {
    if (!active || !threadId || !polling) return
    return visibleTimer(() => void refresh(), POLL_MS)
  }, [active, threadId, polling, refresh])

  const sessions = loaded && loaded.threadId === threadId ? loaded.sessions : []

  const withBusy = async (id: string, work: () => Promise<void>, failure: string) => {
    setBusy((b) => new Set(b).add(id))
    try {
      await work()
    } catch (err) {
      useHQStore.getState().setSystemNotice(errorText(err, failure))
    } finally {
      setBusy((b) => {
        const next = new Set(b)
        next.delete(id)
        return next
      })
    }
  }

  const replaceSessions = (tid: string, update: (list: WatchedSession[]) => WatchedSession[]) =>
    setLoaded((prev) => (prev && prev.threadId === tid ? { threadId: tid, sessions: update(prev.sessions) } : prev))

  const setDrive = (id: string, drive: boolean) =>
    withBusy(
      id,
      async () => {
        const tid = currentThread.current
        const updated = await sessionsApi.setDrive(id, drive)
        if (tid) replaceSessions(tid, (list) => list.map((s) => (s.id === id ? updated : s)))
      },
      drive ? t('chat_say_yes_failed_on') : t('chat_say_yes_failed_off'),
    )

  const unwatch = (id: string) =>
    withBusy(
      id,
      async () => {
        const tid = currentThread.current
        await sessionsApi.unwatch(id)
        if (tid) replaceSessions(tid, (list) => list.filter((s) => s.id !== id))
      },
      t('chat_unwatch_failed'),
    )

  return { sessions, busy, setDrive, unwatch }
}
