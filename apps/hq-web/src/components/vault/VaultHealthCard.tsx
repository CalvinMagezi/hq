import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { AlertTriangle } from 'lucide-react'
import { t, tn } from '~/strings'
import { vaultHealthQuery } from '~/lib/queries'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { problemCount, type VaultHealth } from '~/lib/vaultHealthApi'

interface ViewProps {
  health: VaultHealth | undefined
}

/**
 * A note on Notes home when the check found something to look at, and nothing at all when it
 * did not, when it has not run, or when this HQ cannot answer: a card that is always there
 * becomes furniture, and one that appears means something.
 */
export function VaultHealthCardView({ health }: ViewProps) {
  if (!health?.checked || !health.report) return null
  const c = health.report.counts
  const n = problemCount(c)
  if (n === 0) return null
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
        <Link to="/notes/health" className="a-btn compact secondary">
          {t('notes_health_review')}
        </Link>
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
    </section>
  )
}

/** The card with its data: refetched when a scan or an ignore changes the report. */
export function VaultHealthCard() {
  const q = useQuery(vaultHealthQuery)
  useRefreshOn(['vault-health:change'], () => void q.refetch())
  return <VaultHealthCardView health={q.data} />
}
