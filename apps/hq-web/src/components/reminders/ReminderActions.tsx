import { useQuery } from '@tanstack/react-query'
import { Check } from 'lucide-react'
import { fetchReminders, REMINDERS_KEY } from '~/lib/remindersApi'
import { timeChoices, type TimeChoiceKey } from '~/lib/reminderTimes'
import { displayZone } from '~/lib/time'
import { t, type StringKey } from '~/strings'

export const SNOOZE_LABEL: Record<TimeChoiceKey, StringKey> = {
  hour: 'reminders_snooze_hour',
  evening: 'reminders_snooze_evening',
  tomorrow: 'reminders_snooze_tomorrow',
  monday: 'reminders_snooze_monday',
}

interface Props {
  title: string
  busy: boolean
  /** `done`, or `snooze` with the time it rings again, in epoch seconds. */
  onAct: (action: 'done' | 'snooze', until?: number) => void
}

/** Done and the snooze times for a reminder's ping in the Inbox; the owner's own clock decides "this evening". */
export function ReminderActions({ title, busy, onAct }: Props) {
  const list = useQuery({ queryKey: [...REMINDERS_KEY, 'zone'], queryFn: () => fetchReminders('active'), staleTime: 60_000 })
  const zone = list.data?.time_zone ?? displayZone()
  const choices = timeChoices(Date.now(), zone)
  return (
    <div className="flex flex-wrap items-center gap-2" data-testid="reminder-actions">
      <button type="button" className="a-btn" disabled={busy} aria-label={t('reminders_done_aria', { title })} onClick={() => onAct('done')}>
        <Check className="a-ic sm" aria-hidden="true" />
        {t('reminders_mark_done')}
      </button>
      <span className="a-cap a-muted">{t('reminders_snooze')}</span>
      {choices.map((c) => (
        <button key={c.key} type="button" className="a-btn secondary compact" disabled={busy} aria-label={`${t('reminders_snooze_aria', { title })}: ${t(SNOOZE_LABEL[c.key])}`} onClick={() => onAct('snooze', Math.round(c.at / 1000))}>
          {t(SNOOZE_LABEL[c.key])}
        </button>
      ))}
    </div>
  )
}
