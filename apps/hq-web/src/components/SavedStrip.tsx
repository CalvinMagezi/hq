import { useQueryClient } from '@tanstack/react-query'
import { TriangleAlert } from 'lucide-react'
import type { ConnectionState } from '~/lib/connection'
import { placeSavedAt } from '~/lib/savedCopy'
import { t } from '~/strings'

/** The first query-key parts each place saves; a place missing here keeps no strip. */
const PLACE_KEYS: Record<string, string[]> = {
  '': ['vault'],
  notes: ['vault'],
  settings: ['settings', 'copilot-usage', 'openrouter-usage', 'usage-budgets', 'usage-forecast', 'usage-providers'],
}

export const savedKeysFor = (pathname: string): string[] | null => PLACE_KEYS[pathname.split('/')[1] ?? ''] ?? null

/** Signed out wipes every store, so it has no copy to name; Connected needs no strip. */
export const stripShown = (state: ConnectionState): boolean => state !== 'connected' && state !== 'signed_out'

const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString()

/** "Saved in this browser at 14:32", or "on Tue 3 Nov" for a copy from before today. Uses the browser's own clock setting. */
export function savedLine(savedAt: number | null, now: number = Date.now(), locale?: string): string {
  if (savedAt === null) return t('saved_nothing')
  const when = new Date(savedAt)
  if (sameDay(when, new Date(now))) {
    return t('saved_at', { time: new Intl.DateTimeFormat(locale, { hour: 'numeric', minute: '2-digit' }).format(when) })
  }
  return t('saved_on', { date: new Intl.DateTimeFormat(locale, { weekday: 'short', day: 'numeric', month: 'short' }).format(when) })
}

/** The newest save across the place's query keys. */
export function placeSavedTime(queries: { queryKey: readonly unknown[]; state: { dataUpdatedAt: number } }[], keys: string[]): number | null {
  const times = keys.map((k) => placeSavedAt(queries, k)).filter((x): x is number => x !== null)
  return times.length ? Math.max(...times) : null
}

/** One strip per place under the status line while HQ is not Connected. */
export function SavedStrip({ state, pathname }: { state: ConnectionState; pathname: string }) {
  const client = useQueryClient()
  const keys = savedKeysFor(pathname)
  if (!keys || !stripShown(state)) return null
  const savedAt = placeSavedTime(client.getQueryCache().getAll(), keys)
  return (
    <div className="a-stale" role="status">
      <TriangleAlert className="a-ic" aria-hidden="true" />
      <span>{savedLine(savedAt)}</span>
    </div>
  )
}
