import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Bell, Check, Plus, Repeat, X } from 'lucide-react'
import { PaneHead, Panes } from '~/components/ui/Panes'
import {
  REMINDERS_KEY,
  cancelReminder,
  createReminder,
  doneReminder,
  fetchReminders,
  snoozeReminder,
  type Reminder,
} from '~/lib/remindersApi'
import {
  groupReminders,
  localString,
  parseLocalInput,
  repeatSpec,
  timeChoices,
  type RepeatChoice,
  type ReminderGroup,
  type TimeChoiceKey,
} from '~/lib/reminderTimes'
import { displayZone } from '~/lib/time'
import { toastNotice } from '~/lib/toast'
import { t, type StringKey } from '~/strings'
import { SNOOZE_LABEL } from './ReminderActions'

const POLL_MS = 20_000
const CLOCK_MS = 30_000

const GROUP_LABEL: Record<ReminderGroup, StringKey> = {
  overdue: 'reminders_group_overdue',
  today: 'reminders_group_today',
  tomorrow: 'reminders_group_tomorrow',
  later: 'reminders_group_later',
}

const REPEAT_LABEL: Record<RepeatChoice, StringKey> = {
  none: 'reminders_repeat_none',
  daily: 'reminders_repeat_daily',
  weekdays: 'reminders_repeat_weekdays',
  weekly: 'reminders_repeat_weekly',
  monthly: 'reminders_repeat_monthly',
}

const STATUS_LABEL: Record<string, StringKey> = {
  done: 'reminders_status_done',
  cancelled: 'reminders_status_cancelled',
  lapsed: 'reminders_status_lapsed',
}

type Tab = 'upcoming' | 'done'

/** One change to a reminder: done, snooze or stop. */
interface Job {
  (): Promise<unknown>
}

function useNow(): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), CLOCK_MS)
    return () => window.clearInterval(id)
  }, [])
  return now
}

const reasonOf = (e: unknown) => t('reminders_failed', { reason: (e as { message?: string } | null)?.message ?? '' })

/** The Reminders page: what is coming, what to do about it, and a form for a new one. */
export function RemindersPage() {
  const [tab, setTab] = useState<Tab>('upcoming')
  const [adding, setAdding] = useState(false)
  const now = useNow()
  const list = useQuery({
    queryKey: [...REMINDERS_KEY, tab],
    queryFn: () => fetchReminders(tab === 'upcoming' ? 'active' : 'done'),
    refetchInterval: POLL_MS,
  })
  const qc = useQueryClient()
  const refresh = () => qc.invalidateQueries({ queryKey: REMINDERS_KEY })
  const zone = list.data?.time_zone ?? displayZone()
  const items = list.data?.reminders ?? []
  const groups = groupReminders(items, now, zone)

  return (
    <Panes
      detailLabel={t('reminders_title')}
      detail={
        <>
          <PaneHead title={t('reminders_title')}>
            <button type="button" className="a-btn compact" onClick={() => setAdding((v) => !v)} aria-expanded={adding}>
              <Plus className="a-ic sm" aria-hidden="true" />
              {t('reminders_new')}
            </button>
          </PaneHead>
          <div className="a-tabs" role="tablist" aria-label={t('reminders_title')}>
            {(['upcoming', 'done'] as const).map((id) => (
              <button key={id} type="button" role="tab" className="a-tab" aria-selected={tab === id} onClick={() => setTab(id)}>
                {t(id === 'upcoming' ? 'reminders_tab_upcoming' : 'reminders_tab_done')}
              </button>
            ))}
          </div>
          <div className="hq-inbox-body">
            <div className="a-wrap flex flex-col gap-4">
              {adding && (
                <NewReminderForm
                  zone={zone}
                  onCreated={(r) => {
                    setAdding(false)
                    toastNotice(t('reminders_set_notice', { when: r.due }))
                    void refresh()
                  }}
                />
              )}
              <p className="a-cap a-muted">{list.data?.time_zone ? t('reminders_in_zone', { zone: list.data.time_zone }) : t('reminders_no_zone')}</p>
              {list.isError && <p role="alert" className="a-cap a-muted">{t('reminders_load_failed')}</p>}
              {!list.isError && list.data && items.length === 0 && (
                <div className="a-empty">
                  <span className="a-empty-ic">
                    <Bell className="a-ic" aria-hidden="true" />
                  </span>
                  <p>{tab === 'upcoming' ? t('reminders_empty') : t('reminders_empty_done')}</p>
                </div>
              )}
              {tab === 'upcoming' &&
                groups.map(({ group, items: rows }) => (
                  <section key={group} aria-label={t(GROUP_LABEL[group])} className="flex flex-col gap-2">
                    <h3 className="a-label">{t(GROUP_LABEL[group])}</h3>
                    {rows.map((r) => (
                      <ReminderRow key={r.id} reminder={r} zone={zone} now={now} onChanged={refresh} />
                    ))}
                  </section>
                ))}
              {tab === 'done' && items.map((r) => <FinishedRow key={r.id} reminder={r} />)}
            </div>
          </div>
        </>
      }
    />
  )
}

function Tags({ r }: { r: Reminder }) {
  return (
    <span className="a-cap a-muted flex flex-wrap items-center gap-2">
      {r.repeat && (
        <span className="inline-flex items-center gap-1">
          <Repeat className="a-ic sm" aria-hidden="true" />
          {r.repeat}
        </span>
      )}
      {r.nudge && <span>{t('reminders_keeps_asking')}</span>}
      {r.pings > 0 && <span>{t('reminders_rang', { n: String(r.pings) })}</span>}
      {r.tainted && <span>{t('reminders_set_outside')}</span>}
    </span>
  )
}

function ReminderRow({ reminder: r, zone, now, onChanged }: { reminder: Reminder; zone: string; now: number; onChanged: () => void }) {
  const [failure, setFailure] = useState<string | null>(null)
  const run = useMutation({
    mutationFn: (job: Job) => job(),
    onSuccess: () => {
      setFailure(null)
      onChanged()
    },
    onError: (e) => setFailure(reasonOf(e)),
  })
  const choices = timeChoices(now, zone)
  return (
    <div className="a-card" data-testid="reminder-row">
      <b>{r.title}</b>
      <span className="a-muted">{r.due}</span>
      <Tags r={r} />
      {r.note && <span className="a-body-sm">{r.note}</span>}
      {failure && <p role="alert" className="a-cap a-muted">{failure}</p>}
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" className="a-btn compact" disabled={run.isPending} aria-label={t('reminders_done_aria', { title: r.title })} onClick={() => run.mutate(() => doneReminder(r.id))}>
          <Check className="a-ic sm" aria-hidden="true" />
          {t('reminders_mark_done')}
        </button>
        {choices.map((c) => (
          <button key={c.key} type="button" className="a-btn secondary compact" disabled={run.isPending} aria-label={`${t('reminders_snooze_aria', { title: r.title })}: ${t(SNOOZE_LABEL[c.key])}`} onClick={() => run.mutate(() => snoozeReminder(r.id, Math.round(c.at / 1000)))}>
            {t(SNOOZE_LABEL[c.key])}
          </button>
        ))}
        <button type="button" className="a-ib sm" disabled={run.isPending} aria-label={t('reminders_stop_aria', { title: r.title })} title={t('reminders_stop')} onClick={() => run.mutate(() => cancelReminder(r.id))}>
          <X className="a-ic sm" aria-hidden="true" />
        </button>
      </div>
    </div>
  )
}

function FinishedRow({ reminder: r }: { reminder: Reminder }) {
  return (
    <div className="a-card" data-testid="reminder-finished">
      <b>{r.title}</b>
      <span className="a-muted">{t(STATUS_LABEL[r.status] ?? 'reminders_status_done')}</span>
      <Tags r={r} />
    </div>
  )
}

type WhenChoice = TimeChoiceKey | 'pick'

function NewReminderForm({ zone, onCreated }: { zone: string; onCreated: (r: Reminder) => void }) {
  const [title, setTitle] = useState('')
  const [when, setWhen] = useState<WhenChoice>('hour')
  const [picked, setPicked] = useState('')
  const [repeat, setRepeat] = useState<RepeatChoice>('none')
  const [nudge, setNudge] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const choices = timeChoices(Date.now(), zone, { monday: true })
  const create = useMutation({
    mutationFn: async () => {
      const at = when === 'pick' ? parseLocalInput(picked, zone) : (choices.find((c) => c.key === when)?.at ?? null)
      if (at === null) throw new Error(t('reminders_when_pick'))
      return createReminder({ title: title.trim(), at: localString(at, zone), tz: zone, repeat: repeatSpec(repeat, at, zone), nudge })
    },
    onSuccess: onCreated,
    onError: (e) => setFailure(reasonOf(e)),
  })
  const ready = title.trim() !== '' && (when !== 'pick' || picked !== '')
  return (
    <form
      className="a-card flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault()
        setFailure(null)
        create.mutate()
      }}
    >
      <label className="a-field">
        <span className="a-field-label">{t('reminders_what_label')}</span>
        <input className="a-input" value={title} maxLength={200} onChange={(e) => setTitle(e.target.value)} />
      </label>
      <label className="a-field">
        <span className="a-field-label">{t('reminders_when_label')}</span>
        <select className="a-input" value={when} onChange={(e) => setWhen(e.target.value as WhenChoice)}>
          {choices.map((c) => (
            <option key={c.key} value={c.key}>
              {t(SNOOZE_LABEL[c.key])}
            </option>
          ))}
          <option value="pick">{t('reminders_when_pick')}</option>
        </select>
      </label>
      {when === 'pick' && (
        <label className="a-field">
          <span className="a-sr">{t('reminders_when_pick')}</span>
          <input className="a-input" type="datetime-local" value={picked} onChange={(e) => setPicked(e.target.value)} />
        </label>
      )}
      <label className="a-field">
        <span className="a-field-label">{t('reminders_repeat_label')}</span>
        <select className="a-input" value={repeat} onChange={(e) => setRepeat(e.target.value as RepeatChoice)}>
          {(Object.keys(REPEAT_LABEL) as RepeatChoice[]).map((id) => (
            <option key={id} value={id}>
              {t(REPEAT_LABEL[id])}
            </option>
          ))}
        </select>
      </label>
      <label className="flex items-start gap-2">
        <input type="checkbox" checked={nudge} onChange={(e) => setNudge(e.target.checked)} />
        <span>
          {t('reminders_nudge_label')}
          <span className="a-cap a-muted block">{t('reminders_nudge_note')}</span>
        </span>
      </label>
      {failure && <p role="alert" className="a-cap a-muted">{failure}</p>}
      <div>
        <button type="submit" className="a-btn" disabled={!ready || create.isPending} aria-busy={create.isPending || undefined}>
          {create.isPending ? t('reminders_setting') : t('reminders_set')}
        </button>
      </div>
    </form>
  )
}
