// What the Helpers list shows: the chips, the groups, the one line on each card, and when "Allow once" is a safe button.

import { helperState, type HelperState } from '~/components/sessions/SessionRow'
import type { HarnessSession, PendingDialog } from '~/lib/sessionsApi'
import { parseSqliteUtc } from '~/lib/tasksApi'
import { t } from '~/strings'
import { agentName, computerName, folderName, readDialog } from '~/lib/workbench'
import { isThisComputer } from '~/lib/names'

/** A computer's name in the middle of a sentence: "this computer", or the name HQ knows it by. */
export const computerInLine = (host: string) => (isThisComputer(host) ? t('helpers_this_computer') : computerName(host))

export type HelperChip = 'all' | 'needs' | 'working' | 'finished'
export const HELPER_CHIPS: readonly HelperChip[] = ['all', 'needs', 'working', 'finished']

/** Where each state sits in the list, in the order a person should meet them. */
export type HelperGroup = 'needs' | 'working' | 'ready' | 'offline' | 'lost' | 'past' | 'archived'
export const GROUP_ORDER: readonly HelperGroup[] = ['needs', 'working', 'ready', 'offline', 'lost', 'past', 'archived']

export const GROUP_WORDS: Record<HelperGroup, 'helper_state_needs' | 'helper_state_working' | 'helper_state_ready' | 'helpers_group_offline' | 'helpers_group_lost' | 'helpers_group_past' | 'helpers_group_archived'> = {
  needs: 'helper_state_needs',
  working: 'helper_state_working',
  ready: 'helper_state_ready',
  offline: 'helpers_group_offline',
  lost: 'helpers_group_lost',
  past: 'helpers_group_past',
  archived: 'helpers_group_archived',
}

const CHIP_STATES: Record<HelperChip, readonly HelperState[] | null> = {
  all: null,
  needs: ['needs'],
  working: ['working'],
  finished: ['finished', 'stopped'],
}

export const chipWords = (chip: HelperChip) =>
  ({ all: 'helpers_chip_all', needs: 'helpers_chip_needs', working: 'helpers_chip_working', finished: 'helpers_chip_finished' } as const)[chip]

const inChip = (s: HarnessSession, chip: HelperChip) => {
  const states = CHIP_STATES[chip]
  return states === null || states.includes(helperState(s))
}

/** How many live (not archived) helpers each chip would show. */
export function chipCounts(sessions: HarnessSession[]): Record<HelperChip, number> {
  const live = sessions.filter((s) => !s.archived)
  return Object.fromEntries(HELPER_CHIPS.map((c) => [c, live.filter((s) => inChip(s, c)).length])) as Record<HelperChip, number>
}

const groupOf = (s: HarnessSession): HelperGroup => {
  if (s.archived) return 'archived'
  const state = helperState(s)
  return state === 'finished' || state === 'stopped' ? 'past' : state
}

const activityAt = (s: HarnessSession) => s.last_seen_at ?? s.created_at
const newest = (a: HarnessSession, b: HarnessSession) => activityAt(b).localeCompare(activityAt(a))

/** The chip's helpers split into groups, empty groups left out. Archived ones appear only when asked for. */
export function groupHelpers(sessions: HarnessSession[], chip: HelperChip, showArchived: boolean): { group: HelperGroup; items: HarnessSession[] }[] {
  const visible = sessions.filter((s) => (showArchived || !s.archived) && inChip(s, chip))
  return GROUP_ORDER.map((group) => ({ group, items: visible.filter((s) => groupOf(s) === group).sort(newest) })).filter((g) => g.items.length > 0)
}

/** Helpers that "Archive finished and stopped" would act on: not running, not archived. */
export const finishedToArchive = (sessions: HarnessSession[]) => sessions.filter((s) => !s.archived && s.status !== 'running')

/** "Allow once" shows only when the option under the cursor is a plain yes that is neither remembered nor typed. */
export function canAllowOnce(dialog: PendingDialog): boolean {
  if (dialog.kind === 'question' || dialog.kind === 'trust') return false
  const picked = dialog.options.find((o) => o.selected)
  return Boolean(picked && !picked.remember && !picked.typed && /^yes\b/i.test(picked.label.trim()) && !/,|\band\b/i.test(picked.label))
}

/** "14 min", "3 h" or "2 d" since the helper started, or until it was last seen once it ended. */
export function spanWord(s: Pick<HarnessSession, 'status' | 'created_at' | 'last_seen_at'>, nowMs: number = Date.now()): string {
  const from = parseSqliteUtc(s.created_at).getTime()
  const to = s.status === 'running' || !s.last_seen_at ? nowMs : parseSqliteUtc(s.last_seen_at).getTime()
  const mins = Math.max(0, Math.round((to - from) / 60_000))
  if (mins < 60) return t('helpers_span_min', { n: Math.max(1, mins) })
  const hours = Math.round(mins / 60)
  if (hours < 48) return t('helpers_span_hour', { n: hours })
  return t('helpers_span_day', { n: Math.round(hours / 24) })
}

/** What the helper is doing, in the phone's words. */
export function activityWord(s: HarnessSession): string {
  const state = helperState(s)
  if (state === 'needs') {
    const dialog = readDialog(s)?.dialog
    if (dialog?.kind === 'edit' && dialog.command) return t('helpers_act_wants_edit', { file: dialog.command })
    if (dialog?.kind === 'command') return t('helpers_act_wants_run')
    return t('helpers_act_wants_answer')
  }
  if (state === 'working') return t('helpers_act_working')
  if (state === 'ready') return t('helpers_act_ready')
  if (state === 'lost') return t('helpers_act_lost')
  if (state === 'offline') return t('helpers_act_offline', { computer: computerName(s.host) })
  return t(state === 'finished' ? 'helpers_act_finished' : 'helpers_act_stopped')
}

/** The pieces of a card's line after its state pill: kind, computer, how long, what it is doing. */
export function cardFacts(s: HarnessSession, nowMs: number = Date.now()): string[] {
  return [agentName(s.harness), t('helpers_on_computer', { computer: computerInLine(s.host) }), spanWord(s, nowMs), activityWord(s)]
}

/** The title of the task a filtered list is about, taken from the helpers on it. */
export function taskBanner(sessions: HarnessSession[], taskId: string): { title: string; line: string } {
  const task = sessions.find((s) => s.task?.id === taskId)?.task
  if (!task) return { title: t('helpers_task_showing_plain'), line: t('helpers_task_showing_plain') }
  const name = `${task.display_id} ${task.title}`
  return { title: t('helpers_task_banner', { task: task.display_id }), line: t('helpers_task_showing', { task: name }) }
}

export { folderName }
