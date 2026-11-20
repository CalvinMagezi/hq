import {
  capabilityModeKey,
  starterButtonLabel,
  starterDescription,
  type ParsedSandboxGrant,
  type SandboxGrantProfile,
  type SandboxStarterName,
} from '~/lib/sandboxGrantsApi'
import { profileAllowLines } from '~/components/settings/AccessView'
import { t } from '~/strings'

const REASON_LIMIT = 300

export type SandboxGrantAnswer =
  | { action: 'approve' }
  | { action: 'allow_task' }
  | { action: 'allow_repo' }
  | { action: 'allow_starter'; starter: SandboxStarterName }
  | { action: 'dismiss' }

export interface SandboxGrantActionsProps {
  grant: Pick<ParsedSandboxGrant, 'taskId' | 'repo' | 'suggestedStarter'>
  starters?: SandboxGrantProfile[]
  busy?: boolean
  onAnswer: (answer: SandboxGrantAnswer) => void
}

/** Ordered owner choices for a sandbox access request: most common safe choice first, with starter contents shown beneath the starter option. */
export function SandboxGrantActions({ grant, starters, busy, onAnswer }: SandboxGrantActionsProps) {
  const starterName = grant.suggestedStarter && grant.repo ? grant.suggestedStarter : null
  const starterProfile = starterName ? starters?.find((s) => s.name === starterName) ?? null : null
  const starterLines = starterProfile ? profileAllowLines(starterProfile) : []

  return (
    <div className="flex flex-col gap-2" data-testid="grant-actions">
      <button
        type="button"
        className="a-btn self-start"
        disabled={busy}
        aria-busy={busy || undefined}
        onClick={() => onAnswer({ action: 'approve' })}
      >
        {busy ? t('approval_answering') : t('grants_allow_once')}
      </button>
      {grant.taskId && (
        <button
          type="button"
          className="a-btn secondary self-start"
          disabled={busy}
          onClick={() => onAnswer({ action: 'allow_task' })}
        >
          {t('grants_allow_task')}
        </button>
      )}
      {grant.repo && (
        <button
          type="button"
          className="a-btn secondary self-start"
          disabled={busy}
          onClick={() => onAnswer({ action: 'allow_repo' })}
        >
          {t('grants_allow_repo')}
        </button>
      )}
      {starterName && (
        <div className="flex flex-col gap-1" data-testid="grant-starter-choice">
          <button
            type="button"
            className="a-btn secondary self-start"
            disabled={busy}
            onClick={() => onAnswer({ action: 'allow_starter', starter: starterName })}
          >
            {starterButtonLabel(starterName)}
          </button>
          <div className="flex flex-col gap-0.5 pl-1 a-cap a-muted" data-testid="grant-starter-includes">
            <span>{t('grants_starter_includes', { starter: starterDescription(starterName) })}</span>
            {starterLines.map((line) => (
              <span key={line} className="a-mono break-words">
                {line}
              </span>
            ))}
          </div>
        </div>
      )}
      <button
        type="button"
        className="a-btn secondary self-start"
        disabled={busy}
        onClick={() => onAnswer({ action: 'dismiss' })}
      >
        {t('grants_decline')}
      </button>
    </div>
  )
}

export interface SandboxGrantCardProps {
  grant: ParsedSandboxGrant
  starters?: SandboxGrantProfile[]
  busy?: boolean
  error?: string | null
  onAnswer: (answer: SandboxGrantAnswer) => void
  /** False when the card sits inside an article that already renders the title header. */
  showTitle?: boolean
}

/** Owner decision card for a sandbox-grants request: exact resource and mode, task, repo, expiry, helper's words, and standing-access choices. */
export function SandboxGrantCard({
  grant,
  starters,
  busy,
  error,
  onAnswer,
  showTitle = true,
}: SandboxGrantCardProps) {
  const mode = t(capabilityModeKey(grant.capability))
  const reason = grant.helperReason.trim()
  const clippedReason = reason.length > REASON_LIMIT ? `${reason.slice(0, REASON_LIMIT)}...` : reason

  return (
    <section className="a-card" aria-label={grant.title || t('grants_title')}>
      {showTitle && <p className="text-xs font-semibold">{grant.title || t('grants_title')}</p>}
      <blockquote className="a-quote" data-testid="grant-resource">
        <span className="a-cap a-muted">{mode}</span>{' '}
        <code className="a-mono">{grant.resource}</code>
      </blockquote>
      <div className="flex flex-col gap-1">
        {grant.taskId && (
          <div className="a-drow">
            <span>{t('grants_task_label')}</span>
            <span className="a-mono">{grant.taskId}</span>
          </div>
        )}
        {grant.repo && (
          <div className="a-drow">
            <span>{t('grants_repo_label')}</span>
            <span className="a-mono">{grant.repo}</span>
          </div>
        )}
        <p className="a-cap a-muted" data-testid="grant-expiry">
          {t('grants_expires_day')}
        </p>
      </div>
      {clippedReason && (
        <div className="flex flex-col gap-1" data-testid="grant-helper-reason">
          <p className="a-cap a-muted">{t('grants_helper_words')}</p>
          <blockquote className="a-quote a-muted">{clippedReason}</blockquote>
        </div>
      )}
      <SandboxGrantActions grant={grant} starters={starters} busy={busy} onAnswer={onAnswer} />
      {error && <p role="alert">{error}</p>}
    </section>
  )
}
