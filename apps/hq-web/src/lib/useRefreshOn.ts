import { useEffect } from 'react'
import { useWS } from '~/context/WebSocketContext'

/** Calls `refresh` when the server pushes one of `types`, so a polled panel updates at once and its timer is only a fallback. */
export function useRefreshOn(types: readonly string[], refresh: () => void | Promise<void>): void {
  const { subscribe } = useWS()
  const key = types.join('|')
  useEffect(() => {
    const wanted = key.split('|')
    return subscribe((msg) => {
      if (wanted.includes(msg.type as string)) void refresh()
    })
  }, [subscribe, key, refresh])
}
