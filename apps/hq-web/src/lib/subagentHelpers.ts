// Sub-agent helper runs extracted from chat tool steps (`spawn_subagents`, `agent_delegate`).
// Reflects only what the server actually sent in tool_start / tool_end; never invents missing
// background run status or result links.

import { t } from '~/strings'

export interface ChatHelperRun {
  id: string
  name: string
  goal: string
  status: 'working' | 'done'
  background: boolean
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

/** Formats a summary string for 2+ helpers in a single `spawn_subagents` call. */
export function formatHelpersSummary(helpers: ChatHelperRun[]): string {
  const parts = helpers.slice(0, 3).map((h) => (h.goal ? `${h.name} (${clip(h.goal, SUMMARY_GOAL_MAX)})` : h.name))
  if (helpers.length > 3) parts.push(`+${helpers.length - 3}`)
  return parts.join(', ')
}
