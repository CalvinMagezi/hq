import { useEffect, useRef, useState } from 'react'
import { ChevronDown, ChevronRight, CheckCircle2, Loader2, Code2, Terminal, AlignLeft } from 'lucide-react'
import type { ToolStep } from '~/store/threadStore'
import { CREDITS_TOOLTIP, formatCredits, type StepCredit } from '~/lib/stepCredits'
import { usePersistentToggle } from './usePersistentToggle'

const SUMMARY_TOOL_NAMES = 3
const MS_PER_SECOND = 1000
// Within this many pixels of the list's end counts as following the newest step.
const LIST_FOLLOW_SLACK_PX = 24

function formatDuration(ms: number) {
  return ms < MS_PER_SECOND ? `${ms}ms` : `${(ms / MS_PER_SECOND).toFixed(1)}s`
}

/** "bash ×4, read_file ×3 +2 more", most used first. */
function summarizeTools(steps: ToolStep[]) {
  const counts = new Map<string, number>()
  for (const s of steps) counts.set(s.toolName, (counts.get(s.toolName) ?? 0) + 1)
  const ranked = [...counts.entries()].sort((a, b) => b[1] - a[1])
  const shown = ranked.slice(0, SUMMARY_TOOL_NAMES).map(([name, n]) => (n > 1 ? `${name} ×${n}` : name))
  const rest = ranked.length - SUMMARY_TOOL_NAMES
  return rest > 0 ? `${shown.join(', ')} +${rest} more` : shown.join(', ')
}

/** The one line a running chain shows while collapsed: what is happening right now. */
function liveStatus(steps: ToolStep[]) {
  const running = [...steps].reverse().find((s) => s.status === 'running')
  const latest = running ?? steps[steps.length - 1]
  const note = latest.progressMessages[latest.progressMessages.length - 1]
  return { name: latest.toolName, note, running: !!running }
}

interface Props {
  steps: ToolStep[]
  /** The turn is still streaming, so the header reports progress instead of a summary. */
  live?: boolean
  /** Credits per agent step; each shows on the step's first tool call. */
  credits?: StepCredit[]
}

/**
 * A turn's tool calls as one collapsed row. It never opens by itself: a long
 * chain used to open and close on every step, which made the chat jump.
 */
export function ToolCallBlock({ steps, credits, live = false }: Props) {
  // Keyed on the first call id, which survives the live turn becoming a saved message.
  const [expanded, setExpanded] = usePersistentToggle(`tools:${steps[0]?.toolCallId ?? ''}`, false)
  const [activeStepId, setActiveStepId] = useState<string | null>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const followRef = useRef(true)

  // An open list keeps the newest step in view unless the user scrolled up in it.
  useEffect(() => {
    const el = listRef.current
    if (el && expanded && live && followRef.current) el.scrollTop = el.scrollHeight
  }, [steps, expanded, live])

  if (steps.length === 0) return null

  const done = steps.filter((s) => s.status === 'done').length
  const totalMs = steps.reduce((acc, s) => acc + (s.durationMs ?? 0), 0)
  const status = live ? liveStatus(steps) : null

  return (
    <div className="mb-2 text-xs font-mono rounded-xl overflow-hidden bg-white/[0.025] border border-white/[0.07]">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
        className="w-full flex items-center gap-2 px-3 min-h-10 py-2 text-left transition-colors hover:bg-white/5"
        style={{ color: 'var(--text-dim)' }}
      >
        {live ? (
          <Loader2 className="w-3.5 h-3.5 shrink-0 animate-spin" style={{ color: 'var(--accent-amber)' }} />
        ) : (
          <CheckCircle2 className="w-3.5 h-3.5 shrink-0" style={{ color: 'var(--accent-green)' }} />
        )}
        <span className="min-w-0 flex-1 truncate">
          <span className="font-semibold" style={{ color: 'var(--text-primary)' }}>
            {status ? (status.running ? `${status.name} running` : 'Working') : `${steps.length} tool${steps.length === 1 ? '' : 's'} used`}
          </span>
          <span className="opacity-70">
            {status ? ` · ${done} of ${steps.length} done` : ` · ${summarizeTools(steps)}`}
          </span>
        </span>
        {!live && totalMs > 0 && <span className="text-[11px] opacity-60 shrink-0">{formatDuration(totalMs)}</span>}
        {expanded ? <ChevronDown className="w-3.5 h-3.5 shrink-0" /> : <ChevronRight className="w-3.5 h-3.5 shrink-0" />}
      </button>

      {status && !expanded && (
        <div className="flex items-center gap-1.5 px-3 pb-2 h-6 text-[11px] opacity-80" style={{ color: 'var(--text-dim)' }}>
          <Terminal className="w-2.5 h-2.5 opacity-50 shrink-0" />
          <span className="truncate">{status.note || status.name}</span>
        </div>
      )}

      {expanded && (
        <div
          ref={listRef}
          onScroll={(e) => {
            const el = e.currentTarget
            followRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < LIST_FOLLOW_SLACK_PX
          }}
          className="px-2 sm:px-3 pb-2.5 pt-1 space-y-1.5 border-t border-white/5 max-h-80 overflow-y-auto overscroll-contain"
        >
          {steps.map((step) => (
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
    <div className="rounded-lg bg-black/20 border border-white/[0.04]">
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
