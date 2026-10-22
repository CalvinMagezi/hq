import { expect, test } from 'bun:test'
import type { Calibration, QueueHealth, SweepEstimate, TierCalibration } from '~/lib/verificationApi'
import {
  ageLabel,
  auditEstimate,
  bucketRows,
  calibrationLine,
  countdown,
  criteriaSummary,
  estimateLine,
  levelLabel,
  pickScope,
  queueLines,
  scopeLabel,
  sweepProgress,
  sweepStage,
  verificationActive,
  waitingChips,
} from './verification'

const tier = (over: Partial<TierCalibration>): TierCalibration => ({
  tier: 1,
  audits: 0,
  disagreements: 0,
  disagreement_upper: null,
  escapes: 0,
  state: 'insufficient',
  why: [],
  ...over,
})

const health = (over: Partial<QueueHealth> = {}): QueueHealth => ({
  mode: 'enforce',
  backlog: {},
  owner_by_kind: {},
  oldest_owner_hours: null,
  verdicts_24h: 0,
  completed_24h: 0,
  est_clear_hours: null,
  deferrals: [],
  spent_this_month_usd: 0,
  monthly_ceiling_usd: null,
  alerts: [],
  ...over,
})

test('only the parties with something waiting get a chip, and the owner comes first', () => {
  const chips = waitingChips({ worker: 2, owner: 5, ci: 0, verifier: 1 })
  expect(chips.map((c) => c.who)).toEqual(['owner', 'verifier', 'worker'])
  expect(chips[0]).toMatchObject({ label: 'Needs you', count: 5, yours: true })
  expect(chips.slice(1).every((c) => !c.yours)).toBe(true)
  expect(waitingChips({})).toEqual([])
})

test('the buckets read in the order a person decides them and empty ones are left out', () => {
  const rows = bucketRows({ failed: 3, passed: 10, needs_you: 1, undecidable: 0, unplanned: 4 })
  expect(rows.map((r) => r.bucket)).toEqual(['passed', 'failed', 'needs_you'])
  expect(rows[1].does).toContain('to_do')
  expect(bucketRows(undefined)).toEqual([])
})

test('a sweep offers only the controls that make sense at its stage', () => {
  expect(sweepStage('planning', null).actions).toEqual(['cancel'])
  expect(sweepStage('planned', null).actions).toEqual(['run', 'cancel'])
  expect(sweepStage('running', null).actions).toEqual(['pause', 'cancel'])
  expect(sweepStage('paused', null).actions).toEqual(['run', 'cancel'])
  expect(sweepStage('ready', null).actions).toEqual(['apply', 'cancel'])
  expect(sweepStage('applying', null).actions).toEqual([])
  expect(sweepStage('applied', null).actions).toEqual(['undo'])
  expect(sweepStage('cancelled', null).actions).toEqual([])
  const ready = sweepStage('ready', { will_move: 40, will_keep: 7, confirm_over: 25, needs_confirmation: true })
  expect(ready.hint).toBe('Applying moves 40 tasks and leaves 7 where they are.')
})

test('progress counts the phase that is running and says nothing otherwise', () => {
  const p = { total: 10, buckets: {}, states: { unplanned: 4, planned: 6 } }
  expect(sweepProgress('planning', p)).toMatchObject({ done: 6, percent: 60, label: '6 of 10 planned' })
  const run = { total: 10, buckets: {}, states: { planned: 3, verdict: 7 } }
  expect(sweepProgress('running', run)).toMatchObject({ done: 7, percent: 70, label: '7 of 10 verified' })
  const apply = { total: 10, buckets: {}, states: { verdict: 2, applied: 6, kept: 1, stale: 1 } }
  expect(sweepProgress('applying', apply)).toMatchObject({ done: 8, label: '8 of 10 applied' })
  expect(sweepProgress('ready', run)).toBeNull()
  expect(sweepProgress('planning', { total: 0, buckets: {}, states: {} })).toBeNull()
})

test('the estimate says where its number comes from and warns when no budget is set', () => {
  const e: SweepEstimate = {
    reviewer_runs: 12,
    usd: 0.6,
    minutes: 4,
    basis: 'an assumed $0.05 a run: no reviewer runs to learn from yet',
    reviewer_enabled: false,
    monthly_ceiling_usd: null,
    spent_this_month_usd: 0,
  }
  const line = estimateLine(e) ?? ''
  expect(line).toContain('12 reviewer runs')
  expect(line).toContain('about $0.60 and 4 minutes')
  expect(line).toContain('No reviewer budget is set, so these will wait')
  expect(line).toContain('This is an estimate')
  const funded = estimateLine({ ...e, reviewer_runs: 1, minutes: 1, reviewer_enabled: true, monthly_ceiling_usd: 20, spent_this_month_usd: 3.5 }) ?? ''
  expect(funded).toContain('1 reviewer run,')
  expect(funded).toContain('$3.50 of $20.00 spent this month')
  expect(estimateLine({ ...e, reviewer_runs: 0 })).toBeNull()
  expect(estimateLine(undefined)).toBeNull()
})

test('a scope is offered only when it has what it needs', () => {
  const inputs = { initiativeId: null, selectedIds: [] as string[], since: '' }
  expect(pickScope('all', inputs)).toEqual({ kind: 'all' })
  expect(pickScope('initiative', inputs)).toBeNull()
  expect(pickScope('initiative', { ...inputs, initiativeId: 'in-1' })).toEqual({ kind: 'initiative', id: 'in-1' })
  expect(pickScope('since', { ...inputs, since: 'last week' })).toBeNull()
  expect(pickScope('since', { ...inputs, since: ' 2026-10-01 ' })).toEqual({ kind: 'since', date: '2026-10-01' })
  expect(pickScope('selected', inputs)).toBeNull()
  expect(pickScope('selected', { ...inputs, selectedIds: ['a', 'b'] })).toEqual({ kind: 'tasks', ids: ['a', 'b'] })
  expect(scopeLabel({ kind: 'tasks', ids: ['a'] })).toBe('1 picked task')
  expect(scopeLabel({ kind: 'initiative', id: 'in-1' }, 'Website')).toBe('Everything waiting in Website')
})

test('calibration never states a rate it has no audits for', () => {
  expect(calibrationLine(tier({}))).toBe('Tier 1: not enough data yet. 0 audits, 0 disagreed.')
  expect(calibrationLine(tier({ audits: 40, disagreements: 1, disagreement_upper: 0.08, state: 'earned' }))).toBe(
    'Tier 1: earned. 40 audits, 1 disagreed, upper bound 8%.'
  )
  const cal = (tiers: TierCalibration[]): Calibration => ({
    verifier_config: 'm',
    canaries: { runs: 0, defects: 0, caught: 0, controls: 0, passed: 0, catch_rate: null, false_reject_rate: null },
    tiers,
    open_audits: 0,
  })
  expect(auditEstimate(cal([tier({})]))).toContain('no error estimate')
  expect(auditEstimate(cal([tier({ audits: 12, disagreements: 1 })]))).toContain('Too few to say')
  const enough = auditEstimate(cal([tier({ audits: 35, disagreements: 1, disagreement_upper: 0.12 })]))
  expect(enough).toBe('1 of 35 audits disagreed. HQ is wrong at most 12% of the time, with 95% confidence.')
})

test('the veto countdown reads in the largest sensible unit', () => {
  const now = 1_000_000 * 1000
  expect(countdown(1_000_000 - 5, now)).toBe('now')
  expect(countdown(1_000_000 + 30, now)).toBe('now')
  expect(countdown(1_000_000 + 25 * 60, now)).toBe('25m')
  expect(countdown(1_000_000 + 5 * 3600 + 20 * 60, now)).toBe('5h 20m')
  expect(countdown(1_000_000 + 50 * 3600, now)).toBe('2d 2h')
})

test('ages, levels and criteria read plainly', () => {
  expect(ageLabel(null)).toBe('unknown')
  expect(ageLabel(0)).toBe('under an hour')
  expect(ageLabel(5)).toBe('5h')
  expect(ageLabel(72)).toBe('3 days')
  expect(levelLabel('machine_peer')).toBe('machine checks and an independent review')
  expect(levelLabel('something_new')).toBe('something_new')
  expect(criteriaSummary([])).toBe('No acceptance criteria were stated')
  expect(criteriaSummary([{ text: 'a', result: 'pass' }, { text: 'b', result: 'unclear' }])).toBe('1 of 2 criteria met')
  expect(criteriaSummary([{ text: 'a', result: 'pass' }])).toBe('1 of 1 criterion met')
})

test('the queue lines put the alerts first and say when no model is being called', () => {
  const lines = queueLines(health({ alerts: ['8 items wait on you'], completed_24h: 4, verdicts_24h: 9, est_clear_hours: 2.5, oldest_owner_hours: 100 }))
  expect(lines[0]).toBe('8 items wait on you')
  expect(lines).toContain('4 completed by HQ and 9 verdicts in the last 24 hours.')
  expect(lines).toContain('The queue behind the verifier clears in about 2.5 hours at this rate.')
  expect(lines).toContain('The oldest item waiting on you is 4 days old.')
  expect(lines.at(-1)).toBe('No reviewer budget is set, so no model is called.')
  expect(queueLines(health({ monthly_ceiling_usd: 10, spent_this_month_usd: 2 })).at(-1)).toBe('Reviewer spend this month: $2.00 of $10.00.')
})

test('the screens stay out of the way while verification is off', () => {
  expect(verificationActive('off')).toBe(false)
  expect(verificationActive(undefined)).toBe(false)
  expect(verificationActive('shadow')).toBe(true)
  expect(verificationActive('enforce')).toBe(true)
})
