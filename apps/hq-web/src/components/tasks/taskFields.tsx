import type { ReactNode } from 'react'
import type { TaskPriority } from '~/lib/tasksApi'

export const PRIORITIES: TaskPriority[] = ['urgent', 'high', 'normal', 'low']

/** The small uppercase label above a field in the new-task form. */
export function FieldLabel({ children }: { children: ReactNode }) {
  return (
    <label className="block text-[10px] font-mono font-semibold text-neutral-500 uppercase tracking-wider mb-1">
      {children}
    </label>
  )
}

/** A section heading in the task drawer. */
export function SectionLabel({ children, className = 'mb-1.5' }: { children: ReactNode; className?: string }) {
  return (
    <h3 className={`text-xs font-mono font-semibold text-neutral-400 uppercase tracking-wider ${className}`}>
      {children}
    </h3>
  )
}

interface DateRangeProps {
  start: string
  due: string
  onStart: (v: string) => void
  onDue: (v: string) => void
  inputClass: string
  label: (text: string) => ReactNode
}

/** Start and due date inputs that keep start on or before due. Each sits in its own div for the caller's layout. */
export function DateRangeInputs({ start, due, onStart, onDue, inputClass, label }: DateRangeProps) {
  return (
    <>
      <div>
        {label('Start date')}
        <input
          type="date"
          value={start}
          max={due || undefined}
          onChange={(e) => onStart(e.target.value)}
          className={inputClass}
        />
      </div>
      <div>
        {label('Due date')}
        <input
          type="date"
          value={due}
          min={start || undefined}
          onChange={(e) => onDue(e.target.value)}
          className={inputClass}
        />
      </div>
    </>
  )
}
