import { useState } from 'react'
import { t, tn } from '~/strings'
import { ChevronDown, ChevronRight, CheckCircle2, Code2, Terminal, AlignLeft } from 'lucide-react'
import type { ToolStep } from '~/store/threadStore'
import { CREDITS_TOOLTIP, formatCredits, type StepCredit } from '~/lib/stepCredits'
import { stepLines, stepText } from '~/lib/stepWords'
import { usePersistentToggle } from './usePersistentToggle'

const MS_PER_SECOND = 1000

function formatDuration(ms: number) {
  return ms < MS_PER_SECOND ? `${ms}ms` : `${(ms / MS_PER_SECOND).toFixed(1)}s`
}

interface Props {
  steps: ToolStep[]
  /** Credits per agent step; each shows on the step's first tool call. */
  credits?: StepCredit[]
}

/**
 * A finished reply's tool calls as one collapsed row, "What I did · N steps", under the answer. Open,
 * it lists the steps in plain words; tool names, timings, inputs and outputs sit under Technical
 * details. It never opens by itself. While a reply streams, the live line says what is happening.
 */
export function ToolCallBlock({ steps, credits }: Props) {
  // Keyed on the first call id, which survives the live turn becoming a saved message.
  const [expanded, setExpanded] = usePersistentToggle(`tools:${steps[0]?.toolCallId ?? ''}`, false)
  const [activeStepId, setActiveStepId] = useState<string | null>(null)
  const [technical, setTechnical] = useState(false)
  if (steps.length === 0) return null

  const totalMs = steps.reduce((acc, s) => acc + (s.durationMs ?? 0), 0)
  const lines = stepLines(steps)

  return (
    <div className="mt-2 text-xs rounded-xl overflow-hidden bg-white/[0.025] border border-white/[0.07]">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
        className="w-full flex items-center gap-2 px-3 min-h-10 py-2 text-left transition-colors hover:bg-white/5"
        style={{ color: 'var(--text-dim)' }}
      >
        <CheckCircle2 className="w-3.5 h-3.5 shrink-0" style={{ color: 'var(--accent-green)' }} />
        <span className="min-w-0 flex-1 truncate">
          <span className="font-semibold" style={{ color: 'var(--text-primary)' }}>
            {tn('chat_did', lines.length)}
          </span>
        </span>
        {expanded ? <ChevronDown className="w-3.5 h-3.5 shrink-0" /> : <ChevronRight className="w-3.5 h-3.5 shrink-0" />}
      </button>

      {expanded && (
        <div className="px-2 sm:px-3 pb-2.5 pt-1 space-y-1.5 border-t border-white/5 max-h-80 overflow-y-auto">
          <ul className="text-[13px] space-y-1 py-1" style={{ color: 'var(--text-dim)' }}>
            {lines.map((line, i) => (
              <li key={i} className="flex items-start gap-2 min-h-6">
                <CheckCircle2 className="w-3.5 h-3.5 mt-0.5 shrink-0" style={{ color: 'var(--accent-green)' }} aria-hidden="true" />
                <span className="min-w-0 break-words">{stepText(line)}</span>
              </li>
            ))}
          </ul>
          <button
            type="button"
            onClick={() => setTechnical(!technical)}
            aria-expanded={technical}
            className="flex items-center gap-1 min-h-9 px-1 text-[11px] opacity-70 hover:opacity-100"
            style={{ color: 'var(--text-dim)' }}
          >
            {technical ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
            {t('chat_technical_details')}
            {totalMs > 0 && <span className="opacity-70">{` · ${formatDuration(totalMs)}`}</span>}
          </button>
          {technical &&
            steps.map((step) => (
              <ToolStepRow
                key={step.toolCallId}
                step={step}
                credit={credits?.find((c) => c.toolCallId === step.toolCallId)}
                open={activeStepId === step.toolCallId}
                onToggle={() => setActiveStepId(activeStepId === step.toolCallId ? null : step.toolCallId)}
              />
            ))}
        </div>
      )}
    </div>
  )
}

function ToolStepRow({ step, credit, open, onToggle }: { step: ToolStep; credit?: StepCredit; open: boolean; onToggle: () => void }) {
  const isDone = step.status === 'done'
  const hasDetail = !!step.inputArgs || !!step.resultOutput
  return (
    <div className="rounded-lg bg-black/20 border border-white/[0.04] font-mono">
      <button
        type="button"
        onClick={onToggle}
        disabled={!hasDetail}
        className="w-full flex items-center gap-2 px-2 min-h-9 py-1.5 text-left disabled:cursor-default"
      >
        <span
          className="text-[9px] font-bold px-1.5 py-0.5 rounded uppercase shrink-0"
          style={{
            background: isDone ? 'rgba(0, 255, 163, 0.1)' : 'rgba(255, 179, 0, 0.1)',
            color: isDone ? 'var(--accent-green)' : 'var(--accent-amber)',
          }}
        >
          {step.status}
        </span>
        <span className="font-bold truncate flex-1 min-w-0" style={{ color: 'var(--text-primary)' }}>{step.toolName}</span>
        {credit && (
          <span className="text-[11px] opacity-60 shrink-0" style={{ color: 'var(--text-dim)' }} title={CREDITS_TOOLTIP}>
            {formatCredits(credit.delta)}
          </span>
        )}
        {step.durationMs !== undefined && (
          <span className="text-[11px] opacity-60 shrink-0" style={{ color: 'var(--text-dim)' }}>{formatDuration(step.durationMs)}</span>
        )}
        {hasDetail && (
          <span className="text-[11px] shrink-0" style={{ color: 'var(--text-dim)' }}>{open ? '▲' : '▼'}</span>
        )}
      </button>

      {step.progressMessages.length > 0 && !isDone && (
        <div className="mx-2 mb-1.5 space-y-0.5 pl-2 border-l border-amber-500/20">
          {step.progressMessages.slice(-3).map((msg, idx) => (
            <div key={idx} className="text-[11px] flex items-center gap-1.5" style={{ color: 'var(--text-dim)' }}>
              <Terminal className="w-2.5 h-2.5 opacity-40 shrink-0" />
              <span className="truncate">{msg}</span>
            </div>
          ))}
        </div>
      )}

      {open && (
        <div className="mx-2 mb-2 pt-2 border-t border-white/5 space-y-2 text-[11px]">
          {step.inputArgs && (
            <StepPayload icon={<Code2 className="w-3 h-3" />} label="Input" color="var(--accent-green)">
              {JSON.stringify(step.inputArgs, null, 2)}
            </StepPayload>
          )}
          {step.resultOutput && (
            <StepPayload icon={<AlignLeft className="w-3 h-3" />} label="Output" color="var(--accent-blue)">
              {step.resultOutput}
            </StepPayload>
          )}
        </div>
      )}
    </div>
  )
}

function StepPayload({ icon, label, color, children }: { icon: React.ReactNode; label: string; color: string; children: string }) {
  return (
    <div>
      <div className="flex items-center gap-1 font-semibold mb-1 opacity-75" style={{ color }}>
        {icon} {label}
      </div>
      <pre className="p-2 rounded bg-black/40 border border-white/5 overflow-x-auto text-[11px] leading-tight max-h-48 whitespace-pre-wrap break-words" style={{ color: 'var(--text-primary)' }}>
        {children}
      </pre>
    </div>
  )
}
