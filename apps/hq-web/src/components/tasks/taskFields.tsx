import type { ReactNode } from 'react'
import type { TaskPriority } from '~/lib/tasksApi'

export const PRIORITIES: TaskPriority[] = ['urgent', 'high', 'normal', 'low']

/** The small uppercase label above a field in the new-task form. */
export function FieldLabel({ children }: { children: ReactNode }) {
  return (
    <label className="block text-[11px] font-semibold text-neutral-500 uppercase tracking-wider mb-1">
      {children}
    </label>
  )
}

/** A section heading in the task drawer. */
export function SectionLabel({ children, className = 'mb-1.5' }: { children: ReactNode; className?: string }) {
  return (
    <h3 className={`text-xs font-semibold text-neutral-400 uppercase tracking-wider ${className}`}>
      {children}
    </h3>
  )
}

/** A whole number of minutes, empty for none. Shared by the new-task form and the drawer. */
export function EstimateInput({
  value,
  onChange,
  inputClass,
  label,
}: {
  value: string
  onChange: (v: string) => void
  inputClass: string
  label: (text: string) => ReactNode
}) {
  return (
    <div>
      {label('Estimate (minutes)')}
      <input
        type="number"
        inputMode="numeric"
        min={1}
        step={1}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="none"
        aria-invalid={isInvalidEstimate(value)}
        className={inputClass}
      />
      {isInvalidEstimate(value) && (
        <p role="alert" className="mt-1 text-[11px] text-rose-400">
          Use a whole number of minutes, or leave it empty.
        </p>
      )}
    </div>
  )
}

/** Minutes from an input: a positive whole number, or null when empty or not a valid estimate. */
export function parseEstimate(value: string): number | null {
  const minutes = Number(value)
  return value.trim() !== '' && Number.isInteger(minutes) && minutes > 0 ? minutes : null
}

/** Text that is not empty yet is not a valid estimate, so saving it would silently clear the field. */
export function isInvalidEstimate(value: string): boolean {
  return value.trim() !== '' && parseEstimate(value) === null
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
