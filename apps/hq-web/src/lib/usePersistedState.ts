import { useCallback, useState } from 'react'

type Update<T> = T | ((prev: T) => T)

/** useState kept in localStorage as JSON. Blocked storage or a rejected value falls back to `initial`. */
export function usePersistedState<T>(key: string, initial: T, accept: (v: unknown) => boolean = () => true) {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(key)
      if (raw === null) return initial
      const parsed: unknown = JSON.parse(raw)
      return accept(parsed) ? (parsed as T) : initial
    } catch {
      return initial
    }
  })
  const set = useCallback(
    (next: Update<T>) => {
      setValue((prev) => {
        const v = typeof next === 'function' ? (next as (p: T) => T)(prev) : next
        try {
          localStorage.setItem(key, JSON.stringify(v))
        } catch {
          // Storage can be blocked; the value still holds for this session.
        }
        return v
      })
    },
    [key],
  )
  return [value, set] as const
}
