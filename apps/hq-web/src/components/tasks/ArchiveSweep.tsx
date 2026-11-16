import { useEffect, useState } from 'react'
import { Archive, Loader2 } from 'lucide-react'
import { shelveCompletedClient, type SweepScope } from '~/lib/tasksApi'
import { t, tn } from '~/strings'
import { DEFAULT_SWEEP_DAYS, SWEEP_AGE_OPTIONS } from './shelf'
import './tasks.css'

interface Props {
  scope: SweepScope
  /** Where the sweep reaches, in words: a list, a space, or all lists. */
  label: string
  /** Called with the ids archived, so the page can offer to undo. */
  onArchived: (ids: string[]) => unknown
  onError: (message: string) => void
}

const errorText = (e: unknown) => (e instanceof Error ? e.message : t('tasks_sweep_error'))

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
    <div className="tk-menu-at">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={t('tasks_sweep_hint')}
        className="a-btn compact quiet"
      >
        <Archive className="a-ic xs" aria-hidden="true" />
        {t('tasks_sweep_button')}
      </button>
      {open && (
        <div role="dialog" aria-label={t('tasks_sweep_dialog')} className="a-menu tk-pop a-pad tk-stack">
          <p className="a-body-sm">{t('tasks_sweep_prompt', { label })}</p>
          <select
            value={days}
            onChange={(e) => setDays(Number(e.target.value))}
            aria-label={t('tasks_sweep_days_label')}
            className="a-input"
          >
            {SWEEP_AGE_OPTIONS.map((d) => (
              <option key={d} value={d}>
                {t('tasks_sweep_days_option', { n: d })}
              </option>
            ))}
          </select>
          <p className="a-body-sm a-muted" aria-live="polite">
            {matched === null
              ? t('tasks_sweep_counting')
              : matched === 0
                ? t('tasks_sweep_empty')
                : tn('tasks_sweep_count', matched)}
          </p>
          <div className="tk-add">
            <button type="button" onClick={() => setOpen(false)} className="a-btn compact quiet">
              {t('dl_btn_cancel')}
            </button>
            <button
              type="button"
              onClick={run}
              disabled={running || matched === null || matched === 0}
              className="a-btn compact secondary"
            >
              {running && <Loader2 className="a-ic xs" aria-hidden="true" />}
              {t('tasks_archive_action')}
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
