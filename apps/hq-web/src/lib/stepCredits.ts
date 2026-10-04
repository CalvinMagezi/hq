/** Approximate Copilot credits one agent step used: a before/after difference of an account-wide counter. */
export interface StepCredit {
  turn: number
  /** Null when a reading failed, so the figure is unknown. */
  delta: number | null
  /** The first tool call of this step, where its badge is shown. A step with no tool calls has none. */
  toolCallId?: string
  /** Live only: how many tool steps existed when this arrived, to find the next step's first call. */
  stepsSeen?: number
}

export const CREDITS_TOOLTIP =
  'Approximate. This is the change in one account-wide Copilot counter, so it can include other use of the seat, and GitHub updates it with a delay.'

const TINY_CREDITS = 1

export function formatCredits(delta: number | null | undefined): string {
  if (delta === null || delta === undefined || !Number.isFinite(delta)) return 'n/a'
  return delta < TINY_CREDITS ? '<1 credit' : `~${Math.round(delta)} credits`
}

/** Sum of the known figures; null when none is known, so no total is shown. */
export function totalCredits(credits: StepCredit[] | undefined): number | null {
  const known = (credits ?? []).flatMap((c) => (c.delta === null ? [] : [c.delta]))
  return known.length > 0 ? known.reduce((a, b) => a + b, 0) : null
}

/** The first tool call that belongs to the step a new credit event closes. */
export function firstUnclaimedStep(toolCallIds: string[], credits: StepCredit[]): string | undefined {
  const from = credits[credits.length - 1]?.stepsSeen ?? 0
  return toolCallIds[from]
}

/** A credit entry from a saved reply's meta; a malformed one is dropped. */
export function parseStepCredit(raw: unknown): StepCredit | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  if (typeof r.turn !== 'number') return null
  const delta = typeof r.delta === 'number' ? r.delta : null
  const toolCallId = typeof r.tool_call_id === 'string' ? r.tool_call_id : undefined
  return { turn: r.turn, delta, toolCallId }
}

export function parseStepCredits(raw: unknown): StepCredit[] | undefined {
  if (!Array.isArray(raw)) return undefined
  const parsed = raw.map(parseStepCredit).filter((c): c is StepCredit => c !== null)
  return parsed.length > 0 ? parsed : undefined
}
