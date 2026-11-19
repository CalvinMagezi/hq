// The one live line above a streaming reply: what HQ is doing now, as a sentence, replaced by the next
// step and never stacked. After ten seconds the line offers Stop, so a slow reply always has a way out.
import { t, type StringKey } from '~/strings'
import { stepLine, unwrap, type StepLine } from './stepWords'

/** How long a reply runs before its line offers Stop. */
export const STOP_AFTER_MS = 10_000

interface StepLike {
  toolName: string
  status: 'running' | 'done'
  inputArgs?: Record<string, unknown>
}

/** The sentence for the step running now; null between steps, so the line can say what HQ does instead. */
export function liveSentence(steps: StepLike[]): StepLine | null {
  const running = [...steps].reverse().find((s) => s.status === 'running')
  return running ? stepLine(running.toolName, running.inputArgs) : null
}

const HELPER_TOOLS = new Set(['spawn_subagents', 'agent_delegate', 'harness_session_spawn'])

/**
 * What the line says when no step is running: HQ is waiting on the model, which is never idle time.
 * It names the phase, the steps done so far and the helpers that have reported back.
 */
export function idleLine(steps: StepLike[], thinking: boolean): { key: StringKey; params?: Record<string, string> } {
  if (thinking) return { key: 'chat_live_thinking' }
  const done = steps.filter((s) => s.status === 'done')
  if (done.length === 0) return { key: 'chat_live_reading' }
  const helpers = done.filter((s) => HELPER_TOOLS.has(unwrap(s.toolName, s.inputArgs)[0])).length
  if (helpers > 0) return { key: 'chat_live_after_helpers', params: { n: String(done.length) } }
  return { key: 'chat_live_next_step', params: { n: String(done.length) } }
}

/** Whole seconds a turn has run; never negative, so a clock set back does not show "-3 s". */
export function secondsRunning(startedAt: number | undefined, now: number): number {
  if (startedAt === undefined) return 0
  return Math.max(0, Math.floor((now - startedAt) / 1000))
}

export function stopOffered(startedAt: number | undefined, now: number): boolean {
  return startedAt !== undefined && now - startedAt >= STOP_AFTER_MS
}

/** Each step's line in the present tense. A step with no entry falls back to "HQ is working". */
const LIVE: Partial<Record<StringKey, StringKey>> = {
  step_web_search: 'live_step_web_search',
  step_web_search_for: 'live_step_web_search_for',
  step_web_fetch: 'live_step_web_fetch',
  step_web_fetch_site: 'live_step_web_fetch_site',
  step_web_worked: 'live_step_web_worked',
  step_notes_search: 'live_step_notes_search',
  step_notes_search_for: 'live_step_notes_search_for',
  step_note_read: 'live_step_note_read',
  step_note_read_named: 'live_step_note_read_named',
  step_note_saved: 'live_step_note_saved',
  step_note_saved_named: 'live_step_note_saved_named',
  step_note_updated: 'live_step_note_updated',
  step_note_updated_named: 'live_step_note_updated_named',
  step_note_moved: 'live_step_note_moved',
  step_note_deleted: 'live_step_note_deleted',
  step_notes_looked: 'live_step_notes_looked',
  step_notes_worked: 'live_step_notes_worked',
  step_notes_imported: 'live_step_notes_imported',
  step_document_made: 'live_step_document_made',
  step_document_read: 'live_step_document_read',
  step_picture_read: 'live_step_picture_read',
  step_picture_made: 'live_step_picture_made',
  step_tasks_checked: 'live_step_tasks_checked',
  step_task_added: 'live_step_task_added',
  step_task_added_named: 'live_step_task_added_named',
  step_tasks_added: 'live_step_tasks_added',
  step_tasks_updated: 'live_step_tasks_updated',
  step_task_commented: 'live_step_task_commented',
  step_task_removed: 'live_step_task_removed',
  step_tasks_worked: 'live_step_tasks_worked',
  step_memory_checked: 'live_step_memory_checked',
  step_memory_saved: 'live_step_memory_saved',
  step_chats_searched: 'live_step_chats_searched',
  step_earlier_result: 'live_step_earlier_result',
  step_helper_asked: 'live_step_helper_asked',
  step_helper_started: 'live_step_helper_started',
  step_helper_checked: 'live_step_helper_checked',
  step_command_ran: 'live_step_command_ran',
  step_files_looked: 'live_step_files_looked',
  step_files_changed: 'live_step_files_changed',
  step_code_looked: 'live_step_code_looked',
  step_code_saved: 'live_step_code_saved',
  step_code_worked: 'live_step_code_worked',
  step_google_used: 'live_step_google_used',
  step_owner_notified: 'live_step_owner_notified',
  step_watch_set: 'live_step_watch_set',
  step_watch_stopped: 'live_step_watch_stopped',
  step_watch_checked: 'live_step_watch_checked',
  step_skill_used: 'live_step_skill_used',
  step_skill_changed: 'live_step_skill_changed',
  step_plan_made: 'live_step_plan_made',
  step_progress_shared: 'live_step_progress_shared',
  step_hq_checked: 'live_step_hq_checked',
  step_writing_checked: 'live_step_writing_checked',
  step_used: 'live_step_used',
}

/** The words for a running step: "Reading “Stock rules”…", not "Read “Stock rules”". */
export function liveText(line: StepLine | null): string | null {
  const key = line ? LIVE[line.key] : undefined
  return line && key ? t(key, line.params) : null
}
