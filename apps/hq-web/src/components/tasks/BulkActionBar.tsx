import { useState } from 'react'
import { ChevronUp, Loader2 } from 'lucide-react'
import { STATUS_LABELS, STATUS_ORDER, type TaskPriority, type TaskStatus } from '~/lib/tasksApi'
import { PRIORITIES } from './taskFields'

const priorityLabel = (priority: TaskPriority) => priority.charAt(0).toUpperCase() + priority.slice(1)

interface Props {
  /** Tasks on screen, and how many of them are ticked. */
  total: number
  ticked: number
  /** An action is running: how many requests have settled so far. */
  progress: { settled: number; total: number } | null
  /** What the last action did. Cleared when the selection changes. */
  message: string | null
  onSelectAll: () => void
  onClear: () => void
  onDone: () => void
  onStatus: (status: TaskStatus) => void
  onPriority: (priority: TaskPriority | null) => void
  /** Archives the ticked tasks, or brings them back when the list is the archive. */
  archiveLabel: 'Archive' | 'Unarchive'
  onArchive: () => void
}

type Menu = 'status' | 'priority'

const BUTTON = 'hq-btn-ghost disabled:opacity-40 disabled:pointer-events-none'

// The bar and its menus float over cards, so they need a solid fill, not the frosted one. Inline,
// because the unlayered `.hq-modal` background would beat a Tailwind utility.
const SOLID = { background: 'var(--bg-solid-elevated)' }

/** The strip at the foot of the list while tasks are being picked. */
export function BulkActionBar({ total, ticked, progress, message, onSelectAll, onClear, onDone, onStatus, onPriority, archiveLabel, onArchive }: Props) {
  const [menu, setMenu] = useState<Menu | null>(null)
  const running = progress !== null
  const none = ticked === 0
  const allTicked = total > 0 && ticked === total

  const pick = (apply: () => void) => {
    setMenu(null)
    apply()
  }

  return (
    <div className="sticky bottom-0 z-30 -mx-4 sm:-mx-8 mt-auto px-4 sm:px-8 pt-2 pb-3">
      {menu && <button type="button" aria-label="Close menu" className="fixed inset-0 z-30 cursor-default" onClick={() => setMenu(null)} />}
      <div
        role="toolbar"
        aria-label="Bulk actions"
        style={SOLID}
        className="relative z-40 hq-modal rounded-2xl px-3 py-2.5 flex flex-wrap items-center gap-x-3 gap-y-2"
      >
        <div className="flex items-center gap-2 min-w-0 mr-auto">
          <span className="text-xs font-semibold text-white whitespace-nowrap" aria-live="polite">
            {running ? (
              <span className="flex items-center gap-1.5">
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                {progress.settled} of {progress.total}
              </span>
            ) : (
              `${ticked} selected`
            )}
          </span>
          <button
            type="button"
            disabled={running || total === 0}
            onClick={allTicked ? onClear : onSelectAll}
            className="text-xs text-emerald-400 hover:text-emerald-300 disabled:opacity-40 whitespace-nowrap"
          >
            {allTicked ? 'Clear' : `Select all ${total}`}
          </button>
          {message && !running && <span className="text-xs text-neutral-400 truncate min-w-0">{message}</span>}
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          <MenuButton label="Set status" open={menu === 'status'} disabled={running || none} onToggle={() => setMenu(menu === 'status' ? null : 'status')}>
            {STATUS_ORDER.map((status) => (
              <MenuItem key={status} onClick={() => pick(() => onStatus(status))}>
                {STATUS_LABELS[status]}
              </MenuItem>
            ))}
          </MenuButton>

          <MenuButton label="Set priority" open={menu === 'priority'} disabled={running || none} onToggle={() => setMenu(menu === 'priority' ? null : 'priority')}>
            {PRIORITIES.map((p) => (
              <MenuItem key={p} onClick={() => pick(() => onPriority(p))}>
                {priorityLabel(p)}
              </MenuItem>
            ))}
            <MenuItem onClick={() => pick(() => onPriority(null))}>No priority</MenuItem>
          </MenuButton>

          <button type="button" disabled={running || none} onClick={onArchive} className={BUTTON}>
            {archiveLabel}
          </button>

          <button type="button" disabled={running} onClick={onDone} className={BUTTON}>
            Done
          </button>
        </div>
      </div>
    </div>
  )
}

interface MenuButtonProps {
  label: string
  open: boolean
  disabled: boolean
  onToggle: () => void
  children: React.ReactNode
}

function MenuButton({ label, open, disabled, onToggle, children }: MenuButtonProps) {
  return (
    <div className="relative">
      <button type="button" disabled={disabled} onClick={onToggle} aria-haspopup="menu" aria-expanded={open} className={BUTTON}>
        {label}
        <ChevronUp className={`w-3.5 h-3.5 transition-transform ${open ? '' : 'rotate-180'}`} />
      </button>
      {open && (
        <div role="menu" style={SOLID} className="absolute bottom-full right-0 mb-2 min-w-44 p-1.5 rounded-xl hq-modal flex flex-col">
          {children}
        </div>
      )}
    </div>
  )
}

function MenuItem({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      className="text-left px-3 py-2 rounded-lg text-xs font-semibold text-neutral-200 hover:bg-white/10"
    >
      {children}
    </button>
  )
}
