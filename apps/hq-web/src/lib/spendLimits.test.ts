import { expect, test } from 'bun:test'
import { WORK_KINDS, canSaveLimit, defFromForm, emptyLimitForm, limitChip, scopeString, scopeWords, withLimit } from './spendLimits'
import { spendResetLine } from './time'
import type { BudgetDef } from './usageApi'

test('a scope is chosen from cards and never typed as provider:x', () => {
  expect(scopeString('provider', 'openrouter')).toBe('provider:openrouter')
  expect(scopeWords('provider:openrouter')).not.toContain('provider:')
  expect(scopeWords('global')).not.toContain('global')
})

test('Warn me at is what gets stored in soft_pct', () => {
  const form = { ...emptyLimitForm(), name: 'Month', limit: '50', warn: '65' }
  expect(canSaveLimit(form)).toBe(true)
  expect(defFromForm(form).soft_pct).toEqual([65])
})

test('a limit with no name, no amount or a bad warn level cannot be saved', () => {
  expect(canSaveLimit(emptyLimitForm())).toBe(false)
  expect(canSaveLimit({ ...emptyLimitForm(), name: 'a', limit: '5', warn: '100' })).toBe(false)
})

test('the chip names the period and the percent in words', () => {
  expect(limitChip({ period: 'day', pct: 85 })).toBe('Today 85%')
  expect(limitChip({ period: 'day', pct: 104 })).toBe('Today 104%')
})

test('adding a limit with an existing name replaces it', () => {
  const a: BudgetDef = { name: 'x', scope: 'global', period: 'day', limit_usd: 1, soft_pct: [80], action: 'block', downgrade_model: null }
  expect(withLimit([a], { ...a, limit_usd: 2 }, null)).toHaveLength(1)
})

test('the reset line names midnight UTC', () => {
  expect(spendResetLine('UTC', Date.UTC(2026, 0, 5, 12))).toContain('midnight UTC')
})

test('a work kind names an origin HQ records, so its limit can ever match', () => {
  // hq_llm::origin and harness_usage_collect.rs: what task_outcomes.origin holds.
  const recorded = ['chat', 'background', 'subagent', 'memory', 'skill_review', 'verification', 'compaction', 'embeddings', 'triage', 'imagegen', 'supervisor', 'setup', 'cli', 'harness']
  for (const w of WORK_KINDS) expect(recorded).toContain(w)
})
