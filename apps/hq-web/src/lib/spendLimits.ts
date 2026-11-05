// Spending limits in the owner's words (web wave 4 spec 09): the scope is chosen, never typed, and
// every limit's state reads in grey or amber, never green or red.

import { t, tn, type StringKey } from '~/strings'
import type { BudgetAction, BudgetDef, BudgetPeriod, BudgetStatus } from './usageApi'

export type ScopeKind = 'global' | 'provider' | 'model' | 'origin'

/** The origins HQ files each call under (`hq_llm::origin` and `harness_usage_collect.rs`): a limit's
 * `origin:` must name one of these or it never matches. A chat from a messaging app is `chat` too. */
export const WORK_KINDS = ['chat', 'harness', 'background', 'verification'] as const
export type WorkKind = (typeof WORK_KINDS)[number]

export const WORK_KEY: Record<WorkKind, StringKey> = {
  chat: 'spend_work_chat',
  harness: 'spend_work_helpers',
  background: 'spend_work_scheduled',
  verification: 'spend_work_checks',
}

const SERVICE_NAMES: Record<string, string> = {
  openrouter: 'OpenRouter',
  anthropic: 'Anthropic',
  google: 'Google AI',
  deepseek: 'DeepSeek',
  copilot: 'GitHub Copilot',
  groq: 'Groq',
  openai: 'OpenAI',
}

export const serviceName = (id: string): string => SERVICE_NAMES[id.toLowerCase()] ?? id

export const DEFAULT_WARN_PCT = 80
const HUNDRED = 100

/** `provider:x` and friends, split. An unknown prefix is read as the whole-spend kind. */
export function parseScope(scope: string): { kind: ScopeKind; value: string } {
  const at = scope.indexOf(':')
  const head = at < 0 ? scope : scope.slice(0, at)
  if (head === 'provider' || head === 'model' || head === 'origin') return { kind: head, value: scope.slice(at + 1) }
  return { kind: 'global', value: '' }
}

export function scopeString(kind: ScopeKind, value: string): string {
  return kind === 'global' ? 'global' : `${kind}:${value.trim()}`
}

const lowerFirst = (s: string) => s.charAt(0).toLocaleLowerCase() + s.slice(1)

/** "All AI spending", "OpenRouter only", "claude-haiku-5.5 only", "From scheduled tasks". */
export function scopeWords(scope: string): string {
  const { kind, value } = parseScope(scope)
  if (kind === 'provider') return t('spend_scope_service', { service: serviceName(value) })
  if (kind === 'model') return t('spend_scope_model', { model: value })
  if (kind === 'origin') {
    const work = WORK_KEY[value as WorkKind]
    return t('spend_scope_origin', { origin: work ? lowerFirst(t(work)) : value })
  }
  return t('spend_scope_all')
}

export const PERIOD_KEY: Record<BudgetPeriod, StringKey> = {
  day: 'spend_covers_day',
  week: 'spend_covers_week',
  month: 'spend_covers_month',
}

const CHIP_KEY: Record<BudgetPeriod, StringKey> = {
  day: 'usage_chip_today',
  week: 'usage_chip_week',
  month: 'usage_chip_month',
}

/** The header chip for a limit: "Today 85%". */
export function limitChip(b: Pick<BudgetStatus, 'period' | 'pct'>): string {
  return t('usage_chip', { what: t(CHIP_KEY[b.period]), pct: `${Math.round(b.pct)}%` })
}

export function atLimitWords(b: { action: BudgetAction; downgrade_model?: string | null }): string {
  if (b.action === 'notify') return t('spend_at_notify')
  if (b.action === 'downgrade') return t('spend_at_downgrade', { model: b.downgrade_model ?? '' })
  return t('spend_at_block')
}

/** "Within the limit · resets in 4 days", amber past the warning point, "Over the limit" past 100%. */
export function standingWords(b: Pick<BudgetStatus, 'state' | 'pct'>, spanText: string): string {
  if (b.state === 'exceeded' || b.pct >= HUNDRED) return t('spend_stand_over_resets', { span: spanText })
  if (b.state === 'warning') return t('spend_stand_warn_resets', { span: spanText })
  return t('spend_stand_ok_resets', { span: spanText })
}

export interface LimitForm {
  name: string
  kind: ScopeKind
  value: string
  period: BudgetPeriod
  limit: string
  warn: string
  action: BudgetAction
  downgrade: string
}

export const emptyLimitForm = (): LimitForm => ({
  name: '',
  kind: 'global',
  value: '',
  period: 'month',
  limit: '',
  warn: String(DEFAULT_WARN_PCT),
  action: 'block',
  downgrade: '',
})

export function formFromDef(def: BudgetDef): LimitForm {
  const { kind, value } = parseScope(def.scope)
  return {
    name: def.name,
    kind,
    value,
    period: def.period,
    limit: String(def.limit_usd),
    warn: String(def.soft_pct[0] ?? DEFAULT_WARN_PCT),
    action: def.action,
    downgrade: def.downgrade_model ?? '',
  }
}

export type FormErrors = Partial<Record<'name' | 'value' | 'limit' | 'warn' | 'downgrade', StringKey>>

export function limitErrors(form: LimitForm): FormErrors {
  const out: FormErrors = {}
  if (form.name.trim() === '') out.name = 'spend_err_name'
  if (form.kind !== 'global' && form.value.trim() === '') out.value = 'spend_err_pick'
  const limit = Number(form.limit)
  if (!(form.limit.trim() !== '' && Number.isFinite(limit) && limit > 0)) out.limit = 'spend_err_limit'
  const warn = Number(form.warn)
  if (!(form.warn.trim() !== '' && Number.isInteger(warn) && warn >= 1 && warn < HUNDRED)) out.warn = 'spend_err_warn'
  if (form.action === 'downgrade' && form.downgrade.trim() === '') out.downgrade = 'spend_err_downgrade'
  return out
}

export const canSaveLimit = (form: LimitForm): boolean => Object.keys(limitErrors(form)).length === 0

/** The row HQ stores: `soft_pct` carries what "Warn me at" says. */
export function defFromForm(form: LimitForm): BudgetDef {
  return {
    name: form.name.trim(),
    scope: scopeString(form.kind, form.value),
    period: form.period,
    limit_usd: Number(form.limit),
    soft_pct: [Number(form.warn)],
    action: form.action,
    downgrade_model: form.action === 'downgrade' ? form.downgrade.trim() : null,
  }
}

/** The list after Add or Edit: a limit that was renamed or replaced leaves no twin behind. */
export function withLimit(defs: BudgetDef[], def: BudgetDef, editing: string | null): BudgetDef[] {
  return [...defs.filter((d) => d.name !== def.name && d.name !== editing), def]
}

export function driverLine(answers: number, amount: string): string {
  return tn('spend_driver_line', answers, { amount })
}

const HOURS_PER_DAY = 24

/** "under 1 h", "12 h", "4 days". */
export function spanWords(hours: number): string {
  if (!Number.isFinite(hours) || hours < 1) return t('spend_span_under_hour')
  if (hours < HOURS_PER_DAY) return t('spend_span_hours', { n: Math.round(hours) })
  return tn('spend_span_days', Math.round(hours / HOURS_PER_DAY))
}
