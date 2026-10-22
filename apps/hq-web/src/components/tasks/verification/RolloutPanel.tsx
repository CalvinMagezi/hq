import { AlertTriangle, Check, Circle } from 'lucide-react'
import type { Rollout } from '~/lib/verificationApi'
import { rolloutView } from './verification'

/** Where verification is in being turned on, and what the next step waits for. Read only: the owner changes the setting. */
export function RolloutPanel({ rollout }: { rollout: Rollout }) {
  const v = rolloutView(rollout)
  return (
    <div className="space-y-1.5">
      <p className="text-[11px] font-semibold text-neutral-500 uppercase tracking-wider">Turning it on</p>
      <p className="text-neutral-200">{v.stage}</p>
      {v.warnings.map((w) => (
        <p key={w} className="flex items-start gap-1.5 text-amber-300">
          <AlertTriangle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
          <span>{w}</span>
        </p>
      ))}
      {v.next && (
        <div className="space-y-1">
          <p className="text-neutral-300">
            Next: {v.next.label}. <span className="text-neutral-500">{v.next.verdict}</span>
          </p>
          {v.gates.length > 0 && (
            <ul className="space-y-1">
              {v.gates.map((g) => (
                <li key={g.label} className="flex items-start gap-1.5">
                  {g.met ? (
                    <Check className="w-3.5 h-3.5 mt-0.5 shrink-0 text-emerald-400" />
                  ) : (
                    <Circle className="w-3.5 h-3.5 mt-0.5 shrink-0 text-amber-300" />
                  )}
                  <span className={g.met ? 'text-neutral-500' : 'text-neutral-300'}>
                    {g.label}. <span className="text-neutral-500">{g.detail}</span>
                  </span>
                </li>
              ))}
            </ul>
          )}
          <p className="text-neutral-500">
            Setting: <code className="text-neutral-300">{v.next.change}</code>
          </p>
        </div>
      )}
    </div>
  )
}
