import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { AlertTriangle, CheckCircle2 } from 'lucide-react'
import { t, tn } from '~/strings'
import { vaultHealthQuery } from '~/lib/queries'
import { useHealthChange } from '~/lib/useHealthChange'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { applyHealthFixes, problemCount, undoHealthRepair, undoableRepair, type VaultHealth } from '~/lib/vaultHealthApi'

interface ViewProps {
  health: VaultHealth | undefined
  busy?: boolean
  nowMs?: number
  /** Fix every certain repair. Without it the Fix button is not shown. */
  onFix?: () => void
  onUndo?: (runId: string) => void
}

/**
 * A note on Notes home when the check found something to look at, or when HQ fixed something in
 * the last day and the fix can still be undone. Nothing at all otherwise, when it has not run,
 * or when this HQ cannot answer: a card that is always there becomes furniture, and one that
 * appears means something.
 */
export function VaultHealthCardView({ health, busy = false, nowMs, onFix, onUndo }: ViewProps) {
  if (!health?.checked || !health.report) return null
  const c = health.report.counts
  const n = problemCount(c)
  const undoable = undoableRepair(health, nowMs)
  const certain = health.fix_counts?.a ?? 0

  if (n === 0) {
    if (!undoable) return null
    return (
      <section aria-label={t('notes_health_card_label')} className="a-card mb-4" data-vault-health>
        <div className="l-row justify-between">
          <p className="a-body-sm l-row">
            <CheckCircle2 className="a-ic xs" aria-hidden="true" />
            {tn('notes_health_card_fixed', undoable.fixes)}
          </p>
          {onUndo && (
            <button type="button" className="a-btn compact secondary" disabled={busy} onClick={() => onUndo(undoable.run_id)}>
              {t('notes_health_undo')}
            </button>
          )}
        </div>
      </section>
    )
  }

  const lines: [string, string][] = [
    ['dead', c.links.dead > 0 ? tn('notes_health_dead', c.links.dead) : ''],
    ['ambiguous', c.links.ambiguous > 0 ? tn('notes_health_ambiguous', c.links.ambiguous) : ''],
    ['anchors', c.links.broken_anchors > 0 ? tn('notes_health_anchors', c.links.broken_anchors) : ''],
    ['duplicates', c.duplicates > 0 ? tn('notes_health_duplicates', c.duplicates) : ''],
    ['frontmatter', c.frontmatter > 0 ? tn('notes_health_frontmatter', c.frontmatter) : ''],
  ]
  return (
    <section aria-label={t('notes_health_card_label')} className="a-card mb-4" data-vault-health>
      <div className="l-row justify-between">
        <h2 className="a-h3 l-row">
          <AlertTriangle className="a-ic xs a-warn-t" aria-hidden="true" />
          {tn('notes_health_title', n)}
        </h2>
        <div className="l-row">
          {onFix && certain > 0 && (
            <button type="button" className="a-btn compact" disabled={busy} onClick={onFix}>
              {busy ? t('notes_health_fixing') : tn('notes_health_fix_count', certain)}
            </button>
          )}
          <Link to="/notes/health" className="a-btn compact secondary">
            {t('notes_health_review')}
          </Link>
        </div>
      </div>
      <ul className="l-stack">
        {lines
          .filter(([, text]) => text)
          .map(([id, text]) => (
            <li key={id} className="a-body-sm a-muted">
              {text}
            </li>
          ))}
      </ul>
      {undoable && onUndo && (
        <p className="l-row a-body-sm a-muted">
          {tn('notes_health_card_fixed', undoable.fixes)}
          <button type="button" className="a-link" disabled={busy} onClick={() => onUndo(undoable.run_id)}>
            {t('notes_health_undo')}
          </button>
        </p>
      )}
    </section>
  )
}

/** The card with its data: refetched when a scan, an ignore or a fix changes the report. */
export function VaultHealthCard() {
  const q = useQuery(vaultHealthQuery)
  const { busyKey, change } = useHealthChange()
  useRefreshOn(['vault-health:change'], () => void q.refetch())
  return (
    <VaultHealthCardView
      health={q.data}
      busy={busyKey !== null}
      onFix={() =>
        void change(
          'all',
          async () => {
            const r = await applyHealthFixes({ tier: 'a' })
            return { health: r.health, message: tn('notes_health_fixed', r.applied.fixes.length) }
          },
          t('notes_health_fix_failed'),
        )
      }
      onUndo={(runId) =>
        void change(
          'all',
          async () => {
            const r = await undoHealthRepair(runId)
            const kept = r.undone.edited_since.length
            return {
              health: r.health,
              message: kept > 0 ? tn('notes_health_undo_kept', kept) : tn('notes_health_undone', r.undone.restored.length + r.undone.untrashed.length),
            }
          },
          t('notes_health_undo_failed'),
        )
      }
    />
  )
}
