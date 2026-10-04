import { Brain, ChevronDown, ChevronRight } from 'lucide-react'
import { usePersistentToggle } from './usePersistentToggle'

// The opening of the trace names it; it is the same text before and after the turn is saved.
const MEMORY_KEY_CHARS = 64

/** The last non-empty line of a growing trace, shown while it streams collapsed. */
function latestLine(content: string) {
  const lines = content.trimEnd().split('\n')
  for (let i = lines.length - 1; i >= 0; i--) {
    if (lines[i].trim()) return lines[i].trim()
  }
  return ''
}

export function ThinkingPanel({ content, live = false }: { content: string; live?: boolean }) {
  const [open, setOpen] = usePersistentToggle(`thinking:${content.slice(0, MEMORY_KEY_CHARS)}`, false)
  if (!content || !content.trim()) return null

  return (
    <div className="mb-2.5 rounded-xl overflow-hidden text-xs font-mono border border-amber-500/20 bg-amber-500/[0.03]">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="w-full flex items-center gap-1.5 px-3 min-h-9 py-1.5 text-left transition-colors hover:bg-amber-500/5"
        style={{ color: 'var(--accent-amber)' }}
      >
        <Brain className={`w-3.5 h-3.5 shrink-0 ${live ? 'animate-pulse' : ''}`} />
        <span className="text-[11px] font-bold uppercase tracking-wider shrink-0">
          {live ? 'thinking…' : 'reasoning trace'}
        </span>
        <span className="flex-1 min-w-0 truncate text-[11px] opacity-70 normal-case">
          {live && !open ? latestLine(content) : ''}
        </span>
        <span className="text-[11px] opacity-75 shrink-0">{content.trim().length} chars</span>
        {open ? <ChevronDown className="w-3.5 h-3.5 shrink-0" /> : <ChevronRight className="w-3.5 h-3.5 shrink-0" />}
      </button>

      {open && (
        <div className="px-3 pb-2.5 pt-1 border-t border-amber-500/10 max-h-80 overflow-y-auto overscroll-contain">
          <p className="text-[13px] leading-relaxed whitespace-pre-wrap break-words opacity-80" style={{ color: 'var(--text-dim)' }}>
            {content}
          </p>
        </div>
      )}
    </div>
  )
}
