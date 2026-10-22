import { AlertTriangle } from 'lucide-react'
import type { QueueHealth } from '~/lib/verificationApi'
import { WAITING_LABEL, queueLines, waitingChips } from './verification'

/** Backlog by who it waits on, the sentences that matter, and why anything is deferred. */
export function QueueHealthPanel({ health }: { health: QueueHealth }) {
  const chips = waitingChips(health.backlog)
  const lines = queueLines(health)
  const alerts = health.alerts.length
  return (
    <div className="mt-3 pt-3 border-t border-white/10 space-y-2.5 text-xs">
      {chips.length === 0 ? (
        <p className="text-neutral-400">Nothing is waiting on anyone.</p>
      ) : (
        <ul className="grid grid-cols-2 sm:grid-cols-3 gap-2">
          {chips.map((c) => (
            <li key={c.who} className={`px-3 py-2 rounded-xl border ${c.yours ? 'border-amber-400/30 bg-amber-400/10' : 'border-white/10 bg-white/[0.02]'}`}>
              <span className="block text-lg font-semibold text-white leading-none">{c.count}</span>
              <span className="text-neutral-400">{c.label}</span>
            </li>
          ))}
        </ul>
      )}
      <ul className="space-y-1">
        {lines.map((line, i) => (
          <li key={line} className={i < alerts ? 'flex items-start gap-1.5 text-amber-300' : 'text-neutral-400'}>
            {i < alerts && <AlertTriangle className="w-3.5 h-3.5 mt-0.5 shrink-0" />}
            <span>{line}</span>
          </li>
        ))}
      </ul>
      {health.deferrals.length > 0 && (
        <div>
          <p className="text-[11px] font-semibold text-neutral-500 uppercase tracking-wider mb-1">Waiting, and why</p>
          <ul className="space-y-1">
            {health.deferrals.map((d) => (
              <li key={`${d.waiting_for}-${d.reason}`} className="text-neutral-400">
                <span className="text-neutral-200">{d.tasks}</span> {d.tasks === 1 ? 'task' : 'tasks'} ({WAITING_LABEL[d.waiting_for].toLowerCase()}): {d.reason}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
