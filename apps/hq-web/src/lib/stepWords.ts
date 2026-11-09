// What a reply did, in plain words: one line per step instead of a tool name.
// The phone has the same table in ToolWords (apps/android/.../ui/chat/ChatState.kt); keep the two in step.
import { t, type StringKey } from '~/strings'

/** One line of "What I did": a catalogue key and, when the step named something, that name. */
export interface StepLine {
  key: StringKey
  params?: Record<string, string>
}

type Args = Record<string, unknown> | undefined

/** A step whose line can name what it touched: the key with the name, the key without, and how to find the name. */
interface Named {
  with: StringKey
  without: StringKey
  /** The name's placeholder in the `with` sentence, and how to read it from the arguments. */
  slot: 'query' | 'site' | 'note' | 'title'
}

// Long names are cut so a line fits on a phone.
const MAX_NAME = 60

function short(s: string): string {
  const one = s.replace(/\s+/g, ' ').trim()
  return one.length > MAX_NAME ? `${one.slice(0, MAX_NAME - 1).trimEnd()}…` : one
}

function raw(args: Args, key: string): string | null {
  const v = args?.[key]
  return typeof v === 'string' && v.trim() ? v : null
}

/** "Projects/Plan.md" reads as "Plan". */
function noteName(args: Args): string | null {
  const path = raw(args, 'path')
  if (!path) return null
  const last = path.split('/').filter((p) => p.trim()).pop() ?? ''
  return last.replace(/\.md$/, '') || null
}

/** "https://www.example.com/a/b" reads as "example.com". */
function site(args: Args): string | null {
  const url = raw(args, 'url')
  if (!url) return null
  try {
    return new URL(url.trim()).hostname.replace(/^www\./, '') || null
  } catch {
    return null
  }
}

const PICK: Record<Named['slot'], (args: Args) => string | null> = {
  query: (args) => raw(args, 'query'),
  site,
  note: noteName,
  title: (args) => raw(args, 'title'),
}

const NAMED: Record<string, Named> = {
  web_search: { with: 'step_web_search_for', without: 'step_web_search', slot: 'query' },
  web_fetch: { with: 'step_web_fetch_site', without: 'step_web_fetch', slot: 'site' },
  vault_search: { with: 'step_notes_search_for', without: 'step_notes_search', slot: 'query' },
  vault_find: { with: 'step_notes_search_for', without: 'step_notes_search', slot: 'query' },
  vault_read: { with: 'step_note_read_named', without: 'step_note_read', slot: 'note' },
  vault_read_section: { with: 'step_note_read_named', without: 'step_note_read', slot: 'note' },
  vault_outline: { with: 'step_note_read_named', without: 'step_note_read', slot: 'note' },
  vault_write_note: { with: 'step_note_saved_named', without: 'step_note_saved', slot: 'note' },
  vault_append_note: { with: 'step_note_updated_named', without: 'step_note_updated', slot: 'note' },
  vault_patch_section: { with: 'step_note_updated_named', without: 'step_note_updated', slot: 'note' },
  vault_frontmatter_update: { with: 'step_note_updated_named', without: 'step_note_updated', slot: 'note' },
  task_create: { with: 'step_task_added_named', without: 'step_task_added', slot: 'title' },
  task_create_from_note: { with: 'step_task_added_named', without: 'step_task_added', slot: 'title' },
}

const PLAIN: Record<string, StringKey> = {
  vault_find_similar: 'step_notes_search',
  vault_batch_read: 'step_notes_looked',
  vault_list: 'step_notes_looked',
  vault_tags: 'step_notes_looked',
  vault_links: 'step_notes_looked',
  vault_backlinks: 'step_notes_looked',
  vault_log: 'step_notes_looked',
  vault_context: 'step_notes_looked',
  vault_inventory: 'step_notes_looked',
  context_packet: 'step_notes_looked',
  folder_list: 'step_notes_looked',
  vault_move: 'step_note_moved',
  vault_delete: 'step_note_deleted',
  vault_import: 'step_notes_imported',
  vault_export: 'step_document_made',
  vault_export_pdf: 'step_document_made',
  convert_from_markdown: 'step_document_made',
  convert_to_markdown: 'step_document_read',
  ocr_extract_text: 'step_picture_read',
  generate_image: 'step_picture_made',
  task_list: 'step_tasks_checked',
  task_get: 'step_tasks_checked',
  task_next: 'step_tasks_checked',
  task_related: 'step_tasks_checked',
  task_stale: 'step_tasks_checked',
  task_activity: 'step_tasks_checked',
  task_decisions: 'step_tasks_checked',
  task_time_report: 'step_tasks_checked',
  task_comment_list: 'step_tasks_checked',
  task_link_list: 'step_tasks_checked',
  initiative_list: 'step_tasks_checked',
  initiative_progress: 'step_tasks_checked',
  space_list: 'step_tasks_checked',
  task_create_many: 'step_tasks_added',
  task_update: 'step_tasks_updated',
  task_update_many: 'step_tasks_updated',
  task_claim: 'step_tasks_updated',
  task_release: 'step_tasks_updated',
  task_restore: 'step_tasks_updated',
  task_link_add: 'step_tasks_updated',
  task_link_remove: 'step_tasks_updated',
  initiative_create: 'step_tasks_updated',
  space_create: 'step_tasks_updated',
  space_update: 'step_tasks_updated',
  task_comment_add: 'step_task_commented',
  task_delete: 'step_task_removed',
  memory_entity_graph: 'step_memory_checked',
  vault_person_record_fact: 'step_memory_saved',
  session_search: 'step_chats_searched',
  history_restore: 'step_chats_searched',
  tool_output_read: 'step_earlier_result',
  spawn_subagents: 'step_helper_asked',
  agent_delegate: 'step_helper_asked',
  harness_session_spawn: 'step_helper_started',
  list_agents: 'step_helper_checked',
  bash: 'step_command_ran',
  run_command: 'step_command_ran',
  read_file: 'step_files_looked',
  list_dir: 'step_files_looked',
  find_files: 'step_files_looked',
  grep: 'step_files_looked',
  document_symbols: 'step_files_looked',
  workspace_symbols: 'step_files_looked',
  goto_definition: 'step_files_looked',
  find_references: 'step_files_looked',
  hover: 'step_files_looked',
  write_file: 'step_files_changed',
  edit_file: 'step_files_changed',
  file_edit_batch: 'step_files_changed',
  rollback_file: 'step_files_changed',
  git_status: 'step_code_looked',
  git_diff: 'step_code_looked',
  git_log: 'step_code_looked',
  github_read: 'step_code_looked',
  github_clone: 'step_code_looked',
  git_commit: 'step_code_saved',
  git_pr: 'step_code_saved',
  github_write: 'step_code_saved',
  google_workspace: 'step_google_used',
  notify_owner: 'step_owner_notified',
  watch_create: 'step_watch_set',
  watch_cancel: 'step_watch_stopped',
  watch_list: 'step_watch_checked',
  load_skill: 'step_skill_used',
  list_skills: 'step_skill_used',
  skill_manage: 'step_skill_changed',
  todo_write: 'step_plan_made',
  report_progress: 'step_progress_shared',
  system_info: 'step_hq_checked',
  budget_status: 'step_hq_checked',
  prose_lint: 'step_writing_checked',
}

/** Tools a table above does not name still say roughly what they touched. */
const PREFIXES: [string, StringKey][] = [
  ['vault_', 'step_notes_worked'],
  ['task_', 'step_tasks_worked'],
  ['initiative_', 'step_tasks_worked'],
  ['space_', 'step_tasks_worked'],
  ['harness_session_', 'step_helper_checked'],
  ['host_', 'step_helper_checked'],
  ['subagent_run_', 'step_helper_checked'],
  ['git_', 'step_code_worked'],
  ['github_', 'step_code_worked'],
  ['web_', 'step_web_worked'],
  ['self_update_', 'step_hq_checked'],
]

/** The gateway's `hq_call` carries the real tool in its arguments. */
function unwrap(name: string, args: Args): [string, Args] {
  if (name !== 'hq_call') return [name, args]
  const inner = raw(args, 'tool') ?? raw(args, 'name')
  if (!inner) return [name, args]
  const innerArgs = args?.args ?? args?.arguments
  return [inner, innerArgs && typeof innerArgs === 'object' ? (innerArgs as Record<string, unknown>) : undefined]
}

/** The plain line for one tool call. */
export function stepLine(toolName: string, inputArgs?: Record<string, unknown>): StepLine {
  const [name, args] = unwrap(toolName, inputArgs)
  const named = NAMED[name]
  if (named) {
    const picked = PICK[named.slot](args)
    const value = picked ? short(picked) : ''
    return value ? { key: named.with, params: { [named.slot]: value } } : { key: named.without }
  }
  const plain = PLAIN[name]
  if (plain) return { key: plain }
  const prefix = PREFIXES.find(([p]) => name.startsWith(p))
  if (prefix) return { key: prefix[1] }
  return { key: 'step_used', params: { tool: name.replace(/_/g, ' ').trim() || name } }
}

/** Every step as a line, with a run of the same line said once ("Checked your tasks" three times reads once). */
export function stepLines(steps: { toolName: string; inputArgs?: Record<string, unknown> }[]): StepLine[] {
  const out: StepLine[] = []
  for (const s of steps) {
    const line = stepLine(s.toolName, s.inputArgs)
    const prev = out[out.length - 1]
    if (prev && sameLine(prev, line)) continue
    out.push(line)
  }
  return out
}

function sameLine(a: StepLine, b: StepLine): boolean {
  return a.key === b.key && JSON.stringify(a.params ?? {}) === JSON.stringify(b.params ?? {})
}

export function stepText(line: StepLine): string {
  return t(line.key, line.params)
}
