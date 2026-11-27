import { keepPreviousData, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { SettingsSkeleton } from '~/components/settings/SettingsSkeleton'
import { useSharedConnection } from '~/lib/connection'
import {
  MEMORY_QUERY_KEY,
  editMemory,
  forgetMemory,
  listMemories,
  pinMemory,
  setMemoryStanding,
  type Done,
  type MemoryFilter,
  type Pending,
  type Standing,
} from '~/lib/memoryApi'
import { usePrefs } from '~/lib/prefsApi'
import { t } from '~/strings'
import { MemoryView, type ForgottenNote } from './MemoryView'

const SEARCH_DELAY_MS = 250

/** Settings > What HQ remembers: loads the list and runs each change, then reloads. */
export function MemorySection() {
  const client = useQueryClient()
  const { prefs, set: setPrefValue } = usePrefs()
  const reachable = useSharedConnection() === 'connected'
  const [filter, setFilter] = useState<MemoryFilter>('all')
  const [query, setQuery] = useState('')
  const [searched, setSearched] = useState('')
  const [busyId, setBusyId] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [forgotten, setForgotten] = useState<ForgottenNote | null>(null)

  // A search runs when the person stops typing, not on every key.
  useEffect(() => {
    const id = setTimeout(() => setSearched(query), SEARCH_DELAY_MS)
    return () => clearTimeout(id)
  }, [query])

  const list = useQuery({
    queryKey: [...MEMORY_QUERY_KEY, filter, searched.trim()],
    queryFn: () => listMemories(filter, searched),
    placeholderData: keepPreviousData,
    retry: 1,
  })

  /** Runs one change for one note. Answers whether it worked. */
  const run = async (id: number, change: () => Pending): Done => {
    setBusyId(id)
    setError(null)
    setForgotten(null)
    try {
      await change()
      await client.invalidateQueries({ queryKey: MEMORY_QUERY_KEY })
      return true
    } catch {
      setError(t('memory_action_failed'))
      return false
    } finally {
      setBusyId(null)
    }
  }

  if (!list.data) {
    return list.error ? (
      <div className="a-card" role="status">
        <b>{t('memory_load_failed')}</b>
        <span className="a-muted">{t('settings_failed_body')}</span>
        <button type="button" className="a-btn secondary" onClick={() => void list.refetch()}>
          {t('settings_try_again')}
        </button>
      </div>
    ) : (
      <SettingsSkeleton label={t('settings_loading')} />
    )
  }

  return (
    <MemoryView
      list={list.data}
      filter={filter}
      query={query}
      canChange={reachable}
      busyId={busyId}
      error={error}
      forgotten={forgotten}
      onFilter={(f) => {
        setFilter(f)
        setQuery('')
        setSearched('')
      }}
      onQuery={setQuery}
      onSave={(id, summary) => run(id, () => editMemory(id, summary))}
      onPin={(id, pinned) => void run(id, () => pinMemory(id, pinned))}
      onStanding={(id, standing: Standing) => void run(id, () => setMemoryStanding(id, standing))}
      onForget={async (id) => {
        let notReached = 0
        const ok = await run(id, async () => {
          notReached = (await forgetMemory(id)).not_reached.length
        })
        // What HQ could not reach is said every time, as the server reports it.
        if (ok) setForgotten({ stillThere: notReached > 0 ? ['memory_still_chat', 'memory_still_backups'] : [] })
        return ok
      }}
      learning={prefs?.['memory.learning']}
      onLearning={(on) => setPrefValue({ key: 'memory.learning', value: on })}
    />
  )
}
