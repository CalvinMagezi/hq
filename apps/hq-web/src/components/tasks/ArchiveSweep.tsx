import { useEffect, useState } from 'react'
import { Archive, Loader2 } from 'lucide-react'
import { shelveCompletedClient, type SweepScope } from '~/lib/tasksApi'
import { DEFAULT_SWEEP_DAYS, SWEEP_AGE_OPTIONS } from './shelf'

// A floating panel needs a solid fill: the frosted one lets the cards behind it show through.
const SOLID = { background: 'var(--bg-solid-elevated)' }

interface Props {
  scope: SweepScope
  /** Where the sweep reaches, in words: a list, a space, or all lists. */
  label: string
  /** Called with the ids archived, so the page can offer to undo. */
  onArchived: (ids: string[]) => Promise<void>
  onError: (message: string) => void
}

const errorText = (e: unknown) => (e instanceof Error ? e.message : 'Could not archive')

/**
 * Archives completed tasks older than a chosen age in one step. It counts first and archives
 * only on the second click, so the person always sees how many tasks a sweep will hide.
 */
export function ArchiveSweep({ scope, label, onArchived, onError }: Props) {
  const [open, setOpen] = useState(false)
  const [days, setDays] = useState<number>(DEFAULT_SWEEP_DAYS)
  const [matched, setMatched] = useState<number | null>(null)
  const [running, setRunning] = useState(false)

  useEffect(() => {
    if (!open) return
    let current = true
    setMatched(null)
    shelveCompletedClient(days, true, scope)
      .then((r) => current && setMatched(r.matched))
      .catch((e) => {
        if (!current) return
        setOpen(false)
        onError(errorText(e))
      })
    return () => {
      current = false
    }
    // The scope object is rebuilt each render; its two ids are what matter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, days, scope.space_id, scope.initiative_id])

  const run = async () => {
    setRunning(true)
    try {
      const result = await shelveCompletedClient(days, false, scope)
      setOpen(false)
      await onArchived(result.changed)
    } catch (e) {
      onError(errorText(e))
    } finally {
      setRunning(false)
    }
  }

  return (
    <div className="relative shrink-0">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="dialog"
        aria-expanded={open}
        title="Hide old completed tasks from the lists"
        className="px-2.5 py-1.5 rounded-xl text-xs font-semibold flex items-center gap-1.5 hq-field text-neutral-300 hover:text-white transition-all"
      >
        <Archive className="w-3.5 h-3.5 shrink-0" />
        Tidy up
      </button>
      {open && (
        <>
          <button type="button" aria-label="Close" className="fixed inset-0 z-30 cursor-default" onClick={() => setOpen(false)} />
          <div
            role="dialog"
            aria-label="Archive old completed tasks"
            style={SOLID}
            className="absolute left-0 top-full mt-2 z-40 w-72 max-w-[calc(100vw-2rem)] p-3 rounded-xl hq-modal flex flex-col gap-3"
          >
            <p className="text-xs text-neutral-300">
              Archive completed tasks in <span className="font-semibold text-white">{label}</span> that finished more than
            </p>
            <select
              value={days}
              onChange={(e) => setDays(Number(e.target.value))}
              aria-label="Age in days"
              className="px-2.5 py-1.5 rounded-xl text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400"
            >
              {SWEEP_AGE_OPTIONS.map((d) => (
                <option key={d} value={d}>
                  {d} days ago
                </option>
              ))}
            </select>
            <p className="text-[11px] text-neutral-500" aria-live="polite">
              {matched === null
                ? 'Counting...'
                : matched === 0
                  ? 'Nothing to archive.'
                  : `${matched} task${matched === 1 ? '' : 's'} will leave the lists. Agents still see them, and you can undo this.`}
            </p>
            <div className="flex items-center justify-end gap-2">
              <button type="button" onClick={() => setOpen(false)} className="px-3 py-1.5 rounded-lg text-xs text-neutral-400 hover:text-neutral-200">
                Cancel
              </button>
              <button
                type="button"
                onClick={run}
                disabled={running || matched === null || matched === 0}
                className="hq-btn-ghost disabled:opacity-40 disabled:pointer-events-none"
              >
                {running && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                Archive
              </button>
            </div>
          </div>
        </>
      )}
    </div>
  )
}
