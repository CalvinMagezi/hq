import { useState } from 'react'
import { ChevronDown, Loader2 } from 'lucide-react'
import './tasks.css'
import { STATUS_LABELS, STATUS_ORDER, type TaskPriority, type TaskStatus } from '~/lib/tasksApi'
import { t } from '~/strings'
import { PRIORITIES } from './taskFields'

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
}

type Menu = 'status' | 'priority'

/** The bar that takes the place of the tabs while tasks are being picked. */
export function BulkActionBar({ total, ticked, progress, message, onSelectAll, onClear, onDone, onStatus, onPriority }: Props) {
  const [menu, setMenu] = useState<Menu | null>(null)
  const running = progress !== null
  const none = ticked === 0
  const allTicked = total > 0 && ticked === total

  const pick = (apply: () => void) => {
    setMenu(null)
    apply()
  }

  return (
    <div role="toolbar" aria-label={t('tasks_bulk_bar')} className="tk-bulk">
      <b aria-live="polite">
        {running ? (
          <span className="tk-meta">
            <Loader2 className="a-ic sm" aria-hidden="true" />
            {t('tasks_bulk_progress', { settled: progress.settled, total: progress.total })}
          </span>
        ) : (
          t('tasks_bulk_selected', { n: ticked })
        )}
      </b>
      <button type="button" disabled={running || total === 0} onClick={allTicked ? onClear : onSelectAll} className="a-btn compact quiet">
        {allTicked ? t('inbox_clear') : t('tasks_bulk_select_all', { n: total })}
      </button>
      <MenuButton label={t('tasks_bulk_set_status')} open={menu === 'status'} disabled={running || none} onToggle={() => setMenu(menu === 'status' ? null : 'status')} onClose={() => setMenu(null)}>
        {STATUS_ORDER.map((status) => (
          <MenuItem key={status} onClick={() => pick(() => onStatus(status))}>
            {STATUS_LABELS[status]}
          </MenuItem>
        ))}
      </MenuButton>
      <MenuButton label={t('tasks_bulk_set_priority')} open={menu === 'priority'} disabled={running || none} onToggle={() => setMenu(menu === 'priority' ? null : 'priority')} onClose={() => setMenu(null)}>
        {PRIORITIES.map((p) => (
          <MenuItem key={p} onClick={() => pick(() => onPriority(p))}>
            {t(`tasks_priority_${p}`)}
          </MenuItem>
        ))}
        <MenuItem onClick={() => pick(() => onPriority(null))}>{t('tasks_bulk_no_priority')}</MenuItem>
      </MenuButton>
      <button type="button" disabled={running} onClick={onDone} className="a-btn compact">
        {t('safe_done_button')}
      </button>
      {message && !running && (
        <p className="a-snack" role="status">
          <span>{message}</span>
        </p>
      )}
    </div>
  )
}

interface MenuButtonProps {
  label: string
  open: boolean
  disabled: boolean
  onToggle: () => void
  onClose: () => void
  children: React.ReactNode
}

function MenuButton({ label, open, disabled, onToggle, onClose, children }: MenuButtonProps) {
  return (
    <div className="tk-menu-at" onKeyDown={(e) => e.key === 'Escape' && onClose()}>
      <button type="button" disabled={disabled} onClick={onToggle} aria-haspopup="menu" aria-expanded={open} className="a-btn compact secondary">
        {label}
        <ChevronDown className="a-ic xs" aria-hidden="true" />
      </button>
      {open && (
        <div role="menu" className="a-menu tk-pop">
          {children}
        </div>
      )}
    </div>
  )
}

function MenuItem({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button type="button" role="menuitem" onClick={onClick} className="a-menu-item">
      {children}
    </button>
  )
}
