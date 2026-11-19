// Sub-agent helper runs: prefers the server's run registry (`helperRunsStore`), falling back
// to chat tool steps (`spawn_subagents`, `agent_delegate`) when the endpoint is unavailable.

import { t } from '~/strings'
import type { HelperRun, HelperRunState } from './helperRunsApi'

export type ChatHelperStatus = 'working' | 'done' | 'needs_look' | 'failed'

export interface ChatHelperRun {
  id: string
  name: string
  goal: string
  status: ChatHelperStatus
  background: boolean
  resultLink?: string | null
  error?: string | null
  blocker?: string | null
}

export interface HelperStepLike {
  toolCallId?: string
  toolName: string
  status: 'running' | 'done'
  inputArgs?: Record<string, unknown>
}

const GOAL_MAX = 80
const SUMMARY_GOAL_MAX = 32

function cleanText(v: unknown): string | null {
  if (typeof v !== 'string') return null
  const firstLine = v.trim().split(/\r?\n/)[0]?.trim() ?? ''
  return firstLine || null
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1).trimEnd()}…` : text
}

function unwrapArgs(toolName: string, inputArgs?: Record<string, unknown>): [string, Record<string, unknown> | undefined] {
  if (toolName !== 'hq_call') return [toolName, inputArgs]
  const inner = typeof inputArgs?.tool === 'string' ? inputArgs.tool : typeof inputArgs?.name === 'string' ? inputArgs.name : ''
  if (!inner) return [toolName, inputArgs]
  const innerArgs = inputArgs?.args ?? inputArgs?.arguments
  return [inner, innerArgs && typeof innerArgs === 'object' ? (innerArgs as Record<string, unknown>) : undefined]
}

function parseTaskItem(item: unknown, stepId: string, index: number, status: 'running' | 'done', background: boolean): ChatHelperRun | null {
  if (!item || typeof item !== 'object') return null
  const obj = item as Record<string, unknown>
  const name = cleanText(obj.role) ?? cleanText(obj.id) ?? cleanText(obj.agent) ?? t('chat_helper_default_name')
  const rawGoal = cleanText(obj.prompt) ?? cleanText(obj.goal) ?? cleanText(obj.task)
  const goal = rawGoal ? clip(rawGoal, GOAL_MAX) : ''
  const id = `${stepId}:${cleanText(obj.id) ?? index}`
  return { id, name, goal, status: status === 'running' ? 'working' : 'done', background }
}

/** Extracts sub-agent helper entries from a single `spawn_subagents` or `agent_delegate` tool call. */
export function helpersFromCall(
  toolName: string,
  inputArgs: Record<string, unknown> | undefined,
  status: 'running' | 'done',
  stepId = 'call',
): ChatHelperRun[] {
  const [name, args] = unwrapArgs(toolName, inputArgs)
  if (name !== 'spawn_subagents' && name !== 'agent_delegate') return []
  if (!args) return []

  const background = args.blocking === false
  if (name === 'agent_delegate') {
    const one = parseTaskItem(args, stepId, 0, status, background)
    return one ? [one] : []
  }

  const out: ChatHelperRun[] = []
  if (args.task && typeof args.task === 'object') {
    const one = parseTaskItem(args.task, stepId, out.length, status, background)
    if (one) out.push(one)
  }
  if (Array.isArray(args.tasks)) {
    for (const item of args.tasks) {
      const one = parseTaskItem(item, stepId, out.length, status, background)
      if (one) out.push(one)
    }
  }
  return out
}

/** Collects all sub-agent helper runs across a chat's tool steps, keeping the latest status per id. */
export function extractHelperRuns(steps: HelperStepLike[]): ChatHelperRun[] {
  const out: ChatHelperRun[] = []
  steps.forEach((step, idx) => {
    const stepId = step.toolCallId ?? `step-${idx}`
    for (const run of helpersFromCall(step.toolName, step.inputArgs, step.status, stepId)) {
      out.push(run)
    }
  })
  return out
}

/** Maps a server run state to the four plain-word statuses. */
export function helperStateToStatus(state: HelperRunState): ChatHelperStatus {
  switch (state) {
    case 'queued':
    case 'running':
      return 'working'
    case 'stalled':
    case 'partial':
    case 'blocked':
      return 'needs_look'
    case 'done':
      return 'done'
    case 'failed':
    case 'timed_out':
    case 'cancelled':
    case 'interrupted':
    case 'rejected':
      return 'failed'
  }
}

const STATUS_KEY: Record<ChatHelperStatus, 'chat_helper_working' | 'chat_helper_done' | 'chat_helper_needs_look' | 'chat_helper_failed'> = {
  working: 'chat_helper_working',
  done: 'chat_helper_done',
  needs_look: 'chat_helper_needs_look',
  failed: 'chat_helper_failed',
}

/** Plain-word label for a helper run's status. */
export function helperStatusLabel(status: ChatHelperStatus): string {
  return t(STATUS_KEY[status])
}

/** Converts a server `HelperRun` row into the UI's `ChatHelperRun`. */
export function fromHelperRun(run: HelperRun): ChatHelperRun {
  const name = cleanText(run.role) ?? cleanText(run.task) ?? t('chat_helper_default_name')
  const rawGoal = cleanText(run.goal)
  return {
    id: run.id,
    name,
    goal: rawGoal ? clip(rawGoal, GOAL_MAX) : '',
    status: helperStateToStatus(run.state),
    background: run.detached,
    resultLink: run.result_link,
    error: cleanText(run.error),
    blocker: cleanText(run.blocker),
  }
}

/** Single source of helper runs for a chat: real registry runs when present, else tool-step fallback. */
export function resolveChatHelpers(realRuns: HelperRun[], steps: HelperStepLike[]): ChatHelperRun[] {
  if (realRuns.length > 0) return realRuns.map(fromHelperRun)
  return extractHelperRuns(steps)
}

/** How many helpers are currently working (queued or running). */
export function countRunningHelpers(helpers: ChatHelperRun[]): number {
  return helpers.filter((h) => h.status === 'working').length
}

/** Formats a summary string for 2+ helpers in a single `spawn_subagents` call. */
export function formatHelpersSummary(helpers: ChatHelperRun[]): string {
  const parts = helpers.slice(0, 3).map((h) => (h.goal ? `${h.name} (${clip(h.goal, SUMMARY_GOAL_MAX)})` : h.name))
  if (helpers.length > 3) parts.push(`+${helpers.length - 3}`)
  return parts.join(', ')
}
