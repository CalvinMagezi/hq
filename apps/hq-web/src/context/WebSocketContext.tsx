import { createContext, useContext, useEffect, useRef, useCallback, useState, type ReactNode } from 'react'
import { useHQStore } from '~/store/hqStore'
import { refreshThreads } from '~/lib/threadApi'
import { handleChatEvent } from '~/lib/chatEvents'
import { socketUrl } from '../lib/hqAuth'

type MessageHandler = (msg: Record<string, unknown>) => void

interface WebSocketContextValue {
  /** False when the socket is not open: nothing was sent, so callers must not wait for a reply. */
  send: (data: object) => boolean
  subscribe: (handler: MessageHandler) => () => void
  connected: boolean
}

const WebSocketContext = createContext<WebSocketContextValue | null>(null)

export function WebSocketProvider({ children }: { children: ReactNode }) {
  const { setWsConnected } = useHQStore()
  const wsRef = useRef<WebSocket | null>(null)
  const handlersRef = useRef<Set<MessageHandler>>(new Set())
  const [connected, setConnected] = useState(false)

  useEffect(() => {
    let ws: WebSocket | null = null
    let retryCount = 0
    let retryTimer: ReturnType<typeof setTimeout> | null = null
    let disposed = false
    let fetchingTicket = false

    const scheduleRetry = () => {
      const delay = Math.min(1000 * Math.pow(2, retryCount), 30_000)
      retryCount++
      retryTimer = setTimeout(() => void connect(), delay)
    }

    const connect = async () => {
      const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
      let wsUrl: string
      fetchingTicket = true
      try {
        wsUrl = await socketUrl(`${wsProtocol}//${window.location.host}/ws`)
      } catch {
        if (!disposed) scheduleRetry()
        return
      } finally {
        fetchingTicket = false
      }
      if (disposed) return

      ws = new WebSocket(wsUrl)
      wsRef.current = ws

      const connectTimeout = setTimeout(() => {
        if (ws && ws.readyState === WebSocket.CONNECTING) ws.close()
      }, 10_000)

      ws.onopen = () => {
        clearTimeout(connectTimeout)
        retryCount = 0
        setWsConnected(true)
        setConnected(true)
        // Replies may have finished while the socket was down; their turn_end is gone.
        void refreshThreads().catch(() => {})
      }
      ws.onerror = () => { clearTimeout(connectTimeout) }
      ws.onclose = () => {
        clearTimeout(connectTimeout)
        setWsConnected(false)
        setConnected(false)
        wsRef.current = null
        if (!disposed) scheduleRetry()
      }
      ws.onmessage = (e) => {
        try {
          const msg = JSON.parse(e.data) as Record<string, unknown>
          if (handleChatEvent(msg)) {
            if (msg.type === 'turn_end') for (const handler of handlersRef.current) handler(msg)
            return
          }
          if (msg.type === 'notification:badge_count') {
            const unread = (msg.unread_count as number | undefined) ?? 0
            const pending = (msg.pending_approvals_count as number | undefined) ?? 0
            useHQStore.getState().setNotificationCounts(unread, pending)
          }

          if (msg.type === 'notification:resolved') {
            const id = msg.id as string | undefined
            const newState = (msg.new_state as import('~/lib/notificationsApi').NotificationState | undefined) ?? 'approved'
            if (id) {
              useHQStore.getState().updateNotificationState(id, newState)
            }
          }

          if (msg.type === 'task:created' || msg.type === 'task:updated') {
            const task = msg.task as import('~/lib/tasksApi').TaskItem | undefined
            if (task) useHQStore.getState().upsertTask(task)
          }

          if (msg.type === 'task:deleted') {
            const id = msg.id as string | undefined
            if (id) useHQStore.getState().removeTask(id)
          }

          if (msg.type === 'system:notice') {
            const message = msg.message as string | undefined
            if (message) useHQStore.getState().setSystemNotice(message)
          }

          for (const handler of handlersRef.current) {
            handler(msg)
          }
        } catch (err) {
          // One bad frame must not end the stream, but it must not vanish either.
          console.error('WebSocketProvider: could not handle a message', err)
        }
      }
    }

    // Coming back online or to the foreground: retry now instead of waiting out the backoff.
    const reconnectNow = () => {
      if (document.visibilityState !== 'visible') return
      // A socket that survived the background missed nothing it can prove; reload the list anyway.
      if (ws && ws.readyState === WebSocket.OPEN) {
        void refreshThreads().catch(() => {})
        return
      }
      if (fetchingTicket || (ws && ws.readyState === WebSocket.CONNECTING)) return
      if (retryTimer) clearTimeout(retryTimer)
      retryCount = 0
      void connect()
    }
    window.addEventListener('online', reconnectNow)
    document.addEventListener('visibilitychange', reconnectNow)

    void connect()
    return () => {
      disposed = true
      window.removeEventListener('online', reconnectNow)
      document.removeEventListener('visibilitychange', reconnectNow)
      if (retryTimer) clearTimeout(retryTimer)
      ws?.close()
    }
  }, [setWsConnected])

  const send = useCallback((data: object) => {
    if (wsRef.current?.readyState !== WebSocket.OPEN) return false
    wsRef.current.send(JSON.stringify(data))
    return true
  }, [])

  const subscribe = useCallback((handler: MessageHandler) => {
    handlersRef.current.add(handler)
    return () => { handlersRef.current.delete(handler) }
  }, [])

  return (
    <WebSocketContext.Provider value={{ send, subscribe, connected }}>
      {children}
    </WebSocketContext.Provider>
  )
}

export function useWS() {
  const ctx = useContext(WebSocketContext)
  if (!ctx) throw new Error('useWS must be used within WebSocketProvider')
  return ctx
}
