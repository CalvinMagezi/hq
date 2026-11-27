// The Reminders routes (`/api/reminders`, `crates/hq-web/src/reminders_api.rs`).

import { hqJson } from './hqAuth'

export type ReminderStatus = 'active' | 'done' | 'cancelled' | 'lapsed'

export interface Reminder {
  id: string
  title: string
  note: string | null
  status: ReminderStatus
  /** The next ring in the owner's words: "4:00 pm today". */
  due: string
  /** The next ring, in epoch seconds. */
  due_epoch: number
  time_zone: string
  /** "every day", "every Mon, Thu" and so on; null for a reminder that rings once. */
  repeat: string | null
  /** Keeps reminding until it is marked done. */
  nudge: boolean
  task_id: string | null
  /** How many times it has rung. */
  pings: number
  tainted: boolean
  source: string
}

export interface RemindersResponse {
  /** The zone the owner saved; null until a phone reports one or they choose it. */
  time_zone: string | null
  /** The server's clock, in epoch seconds. */
  now: number
  reminders: Reminder[]
}

export interface NewReminder {
  title: string
  /** Local time in `tz`, `YYYY-MM-DDTHH:MM`. */
  at: string
  tz: string
  repeat?: string
  nudge?: boolean
}

export const REMINDERS_KEY = ['reminders'] as const

export function fetchReminders(status: 'active' | 'done' = 'active'): Promise<RemindersResponse> {
  return hqJson(`/api/reminders?status=${status}`)
}

export function createReminder(body: NewReminder): Promise<Reminder> {
  return hqJson('/api/reminders', 'POST', body)
}

export function doneReminder(id: string): Promise<Reminder> {
  return hqJson(`/api/reminders/${encodeURIComponent(id)}/done`, 'POST')
}

/** Rings again at `until` (epoch seconds). */
export function snoozeReminder(id: string, until: number): Promise<Reminder> {
  return hqJson(`/api/reminders/${encodeURIComponent(id)}/snooze`, 'POST', { until })
}

export function cancelReminder(id: string): Promise<void> {
  return hqJson(`/api/reminders/${encodeURIComponent(id)}`, 'DELETE')
}
