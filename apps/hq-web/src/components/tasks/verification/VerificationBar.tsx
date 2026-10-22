import { useState } from 'react'
import { ChevronDown, ClipboardCheck, Gauge, ShieldCheck } from 'lucide-react'
import { fetchVerificationStatus, type VerificationStatus } from '~/lib/verificationApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../../sessions/usePolled'
import { QueueHealthPanel } from './QueueHealthPanel'
import { verificationActive, waitingChips } from './verification'

const STATUS_POLL_MS = 30_000

interface ViewProps {
  status: VerificationStatus
  healthOpen: boolean
  onToggleHealth: () => void
  onVerifyNow: () => void
  onAudit: () => void
}

/** Counts by who tasks wait on, and the three things the owner does here: verify now, audit, read the queue. */
export function VerificationBarView({ status, healthOpen, onToggleHealth, onVerifyNow, onAudit }: ViewProps) {
  const chips = waitingChips(status.queue.backlog)
  const watching = status.mode === 'shadow'
  const audits = status.calibration.open_audits
  return (
    <section aria-label="Verification" className="mb-4 rounded-2xl hq-glass-card px-3.5 py-3">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <div className="flex items-center gap-2 min-w-0 mr-auto">
          <ShieldCheck className={`w-4 h-4 shrink-0 ${watching ? 'text-amber-300' : 'text-emerald-400'}`} />
          <span className="text-xs font-semibold text-white">Verification</span>
          {watching && (
            <span className="text-[11px] px-1.5 py-0.5 rounded-md bg-amber-400/15 text-amber-300" title="Verdicts are recorded and nothing is moved">
              watching only
            </span>
          )}
        </div>
        <ul className="flex flex-wrap items-center gap-1.5">
          {chips.length === 0 && <li className="text-xs text-neutral-500">Nothing waiting</li>}
          {chips.map((c) => (
            <li
              key={c.who}
              className={`text-xs px-2 py-1 rounded-lg ${c.yours ? 'bg-amber-400/15 text-amber-200 font-semibold' : 'bg-white/5 text-neutral-300'}`}
            >
              {c.count} {c.label.toLowerCase()}
            </li>
          ))}
        </ul>
        <div className="flex items-center gap-1.5">
          <button type="button" onClick={onVerifyNow} className="hq-btn-primary !h-10 !px-3.5 !text-xs">
            <ClipboardCheck className="w-3.5 h-3.5" />
            Verify now
          </button>
          <button type="button" onClick={onAudit} className="hq-btn-ghost !h-10 !px-3 !text-xs">
            Audit{audits > 0 ? ` (${audits})` : ''}
          </button>
          <button
            type="button"
            onClick={onToggleHealth}
            aria-expanded={healthOpen}
            aria-label="Queue health"
            className="hq-btn-ghost !h-10 !px-2.5"
          >
            <Gauge className="w-3.5 h-3.5" />
            <ChevronDown className={`w-3.5 h-3.5 transition-transform ${healthOpen ? 'rotate-180' : ''}`} />
          </button>
        </div>
      </div>
      {healthOpen && <QueueHealthPanel health={status.queue} />}
    </section>
  )
}

interface Props {
  onVerifyNow: () => void
  onAudit: () => void
}

/** Shown on the tasks page once verification is on; nothing at all while it is off. */
export function VerificationBar({ onVerifyNow, onAudit }: Props) {
  const status = usePolled('verification-status', fetchVerificationStatus, STATUS_POLL_MS)
  useRefreshOn(['task:sync'], status.refresh)
  const [healthOpen, setHealthOpen] = useState(false)
  if (!status.data || !verificationActive(status.data.mode)) return null
  return (
    <VerificationBarView
      status={status.data}
      healthOpen={healthOpen}
      onToggleHealth={() => setHealthOpen((v) => !v)}
      onVerifyNow={onVerifyNow}
      onAudit={onAudit}
    />
  )
}
