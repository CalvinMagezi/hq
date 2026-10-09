import { useEffect } from 'react'
import { useHQStore } from '~/store/hqStore'
import { globalSessionsApi } from '~/lib/sessionsApi'
import { needsYouCount } from '~/lib/workbench'
import { usePolled } from './usePolled'

const COUNT_POLL_MS = 30_000

/** Keeps the nav badge fresh from anywhere in the app. Idle while the Workbench list is open, since that list feeds the count. */
export function useNeedsYouCount() {
  const setCount = useHQStore((s) => s.setNeedsYouCount)
  const fed = useHQStore((s) => s.workbenchFeedsCount)
  const list = usePolled('nav-count', () => globalSessionsApi.list(), COUNT_POLL_MS, !fed)
  const count = list.data ? needsYouCount(list.data) : null
  const failed = list.error !== null && !list.loading
  useEffect(() => {
    if (fed) return
    if (failed) setCount(0)
    else if (count !== null) setCount(count)
  }, [fed, failed, count, setCount])
}
