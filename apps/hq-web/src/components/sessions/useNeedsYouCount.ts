import { useEffect } from 'react'
import { useHQStore } from '~/store/hqStore'
import { globalSessionsApi } from '~/lib/sessionsApi'
import { needsYouCount } from '~/lib/workbench'
import { usePolled } from './usePolled'

const COUNT_POLL_MS = 15_000

/** Keeps the nav badge's count of agents waiting on the person fresh from anywhere in the app. */
export function useNeedsYouCount() {
  const setCount = useHQStore((s) => s.setNeedsYouCount)
  const list = usePolled('nav-count', () => globalSessionsApi.list(), COUNT_POLL_MS)
  const count = list.data ? needsYouCount(list.data) : null
  useEffect(() => {
    if (count !== null) setCount(count)
  }, [count, setCount])
}
