import { createFileRoute } from '@tanstack/react-router'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState, type ReactNode } from 'react'
import { AlertCircle, Loader2, RefreshCw, Trash2 } from 'lucide-react'
import {
  budgetsQuery,
  exhaustionSentence,
  forecastQuery,
  pct,
  providersQuery,
  saveBudgets,
  span,
  usd,
  type BudgetAction,
  type BudgetDef,
  type BudgetPeriod,
  type BudgetStatus,
  type Driver,
  type ProviderRow,
} from '~/lib/usageApi'

export const Route = createFileRoute('/usage')({
  component: UsagePage,
})

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="glass-card rounded-2xl p-4 border border-white/10">
      <h2 className="text-[11px] font-mono uppercase tracking-wider text-neutral-400 mb-3">{title}</h2>
      <div className="flex flex-col gap-2">{children}</div>
    </section>
  )
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col sm:flex-row sm:items-baseline sm:justify-between gap-0.5 sm:gap-4 text-xs">
      <span className="text-neutral-400 flex-shrink-0">{label}</span>
      <span className="font-mono text-neutral-100 sm:text-right break-all min-w-0">{children}</span>
    </div>
  )
}

const MAX_BAR_PCT = 100

function Bar({ value, exceeded }: { value: number; exceeded: boolean }) {
  return (
    <div className="h-2 rounded-full overflow-hidden" style={{ background: 'var(--border)' }}>
      <div
        className={`h-full rounded-full ${exceeded ? 'bg-red-400' : ''}`}
        style={{ width: `${Math.min(MAX_BAR_PCT, Math.max(0, value))}%`, ...(exceeded ? {} : { background: 'var(--accent-blue)' }) }}
      />
    </div>
  )
}

function resetText(resetsAt: number): string {
  const hours = (resetsAt * 1000 - Date.now()) / 3_600_000
  return `resets in ${span(Math.max(0, hours))}`
}

function BudgetRow({ b, onDelete }: { b: BudgetStatus; onDelete: () => void }) {
  return (
    <div className="flex flex-col gap-1" data-testid="budget-row">
      <div className="flex items-baseline justify-between gap-2 text-xs">
        <span className="text-neutral-100">
          {b.name} <span className="text-neutral-400 font-mono">{b.scope}, {b.period}ly, then {b.action}</span>
        </span>
        <span className="font-mono text-neutral-100 flex items-center gap-2">
          {usd(b.spent_usd)} of {usd(b.limit_usd)} ({pct(b.pct)})
          <button type="button" onClick={onDelete} aria-label={`Delete budget ${b.name}`} className="text-neutral-400 hover:text-neutral-100">
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </span>
      </div>
      <Bar value={b.pct} exceeded={b.state === 'exceeded'} />
      <span className="text-[11px] text-neutral-400">
        {b.state === 'exceeded' ? 'Limit reached' : b.state === 'warning' ? 'Past the alert threshold' : 'Within budget'}, {resetText(b.resets_at)}
      </span>
    </div>
  )
}

const EMPTY_FORM = { name: '', scope: 'global', period: 'month' as BudgetPeriod, limit: '', action: 'block' as BudgetAction, downgrade: '' }

function BudgetsPanel() {
  const qc = useQueryClient()
  const { data, isLoading } = useQuery(budgetsQuery)
  const [form, setForm] = useState(EMPTY_FORM)
  const save = useMutation({
    mutationFn: (defs: BudgetDef[]) =>
      saveBudgets(defs, { background_run_usd: data?.background_run_usd ?? null, allow_unpriced_models: data?.allow_unpriced_models ?? [] }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['usage-budgets'] }),
  })
  const defs = data?.definitions ?? []
  const limit = Number(form.limit)
  const valid = form.name.trim() !== '' && Number.isFinite(limit) && limit > 0 && (form.action !== 'downgrade' || form.downgrade.trim() !== '')
  const add = () => {
    const def: BudgetDef = {
      name: form.name.trim(),
      scope: form.scope.trim() || 'global',
      period: form.period,
      limit_usd: limit,
      soft_pct: [80],
      action: form.action,
      downgrade_model: form.action === 'downgrade' ? form.downgrade.trim() : null,
    }
    save.mutate([...defs.filter((d) => d.name !== def.name), def], { onSuccess: () => setForm(EMPTY_FORM) })
  }
  const input = 'bg-transparent border border-white/10 rounded-md px-2 py-1 text-xs text-neutral-100 font-mono min-w-0'
  return (
    <Section title="Budgets">
      {isLoading && <div className="flex items-center gap-2 text-xs text-neutral-400"><Loader2 className="w-4 h-4 animate-spin" /> Loading budgets</div>}
      {data?.problems.map((p) => (
        <div key={p} className="flex items-center gap-2 text-xs text-red-400" role="alert"><AlertCircle className="w-4 h-4" /> {p}</div>
      ))}
      {data && data.budgets.length === 0 && (
        <p className="text-xs text-neutral-400">No budgets are set, so nothing is limited. Add one below; it applies to every call straight away.</p>
      )}
      {data?.budgets.map((b) => (
        <BudgetRow key={b.name} b={b} onDelete={() => save.mutate(defs.filter((d) => d.name !== b.name))} />
      ))}
      <form
        className="grid grid-cols-2 sm:grid-cols-6 gap-2 items-end pt-2 border-t border-white/10"
        onSubmit={(e) => { e.preventDefault(); if (valid) add() }}
      >
        <label className="flex flex-col gap-1 text-[11px] text-neutral-400">Name
          <input className={input} value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
        </label>
        <label className="flex flex-col gap-1 text-[11px] text-neutral-400">Scope
          <input className={input} value={form.scope} placeholder="global, provider:x, model:x, origin:x" onChange={(e) => setForm({ ...form, scope: e.target.value })} />
        </label>
        <label className="flex flex-col gap-1 text-[11px] text-neutral-400">Period
          <select className={input} value={form.period} onChange={(e) => setForm({ ...form, period: e.target.value as BudgetPeriod })}>
            <option value="day">day</option><option value="week">week</option><option value="month">month</option>
          </select>
        </label>
        <label className="flex flex-col gap-1 text-[11px] text-neutral-400">Limit (USD)
          <input className={input} inputMode="decimal" value={form.limit} onChange={(e) => setForm({ ...form, limit: e.target.value })} />
        </label>
        <label className="flex flex-col gap-1 text-[11px] text-neutral-400">At the limit
          <select className={input} value={form.action} onChange={(e) => setForm({ ...form, action: e.target.value as BudgetAction })}>
            <option value="block">block</option><option value="downgrade">downgrade</option><option value="notify">notify</option>
          </select>
        </label>
        <button type="submit" disabled={!valid || save.isPending} className="px-2 py-1 rounded-md border border-white/10 text-xs text-neutral-100 hover:bg-white/5 disabled:opacity-40">
          {save.isPending ? 'Saving' : 'Save budget'}
        </button>
        {form.action === 'downgrade' && (
          <label className="flex flex-col gap-1 text-[11px] text-neutral-400 col-span-2 sm:col-span-3">Cheaper model
            <input className={input} value={form.downgrade} onChange={(e) => setForm({ ...form, downgrade: e.target.value })} />
          </label>
        )}
      </form>
      {save.isError && (
        <div className="flex items-center gap-2 text-xs text-red-400" role="alert"><AlertCircle className="w-4 h-4" /> {save.error instanceof Error ? save.error.message : 'Could not save'}</div>
      )}
    </Section>
  )
}

function DriverTable({ title, rows }: { title: string; rows: Driver[] }) {
  if (rows.length === 0) return null
  return (
    <div className="flex flex-col gap-1">
      <h3 className="text-[11px] text-neutral-400">{title}</h3>
      {rows.map((d) => (
        <div key={d.name} className="flex justify-between gap-3 text-xs">
          <span className="text-neutral-100 truncate min-w-0">{d.name}</span>
          <span className="font-mono text-neutral-400 flex-shrink-0">
            {usd(d.usd)}, {d.calls} calls, avg prompt {Math.round(d.avg_input_tokens)} tokens, cache {d.cache_hit_rate === null ? '-' : pct(d.cache_hit_rate * 100)}
          </span>
        </div>
      ))}
    </div>
  )
}

function ForecastPanel() {
  const { data, isLoading } = useQuery(forecastQuery)
  const m = data?.month
  return (
    <Section title="Forecast, this month">
      {isLoading && <div className="flex items-center gap-2 text-xs text-neutral-400"><Loader2 className="w-4 h-4 animate-spin" /> Loading forecast</div>}
      {m && m.rate_per_hour === null && <p className="text-xs text-neutral-400">No spend recorded yet, so there is nothing to forecast.</p>}
      {m && m.rate_per_hour !== null && (
        <>
          <Row label="Spent so far">{usd(m.spent_this_period)}</Row>
          <Row label="Burn rate now">{usd(m.rate_per_hour)} per hour ({m.confidence} confidence)</Row>
          <Row label="Projected month end">{usd(m.projected_period_total)}</Row>
          <Row label="Resets in">{span(m.hours_to_reset)}</Row>
          {exhaustionSentence(m) && <Row label="Budget">{exhaustionSentence(m)}</Row>}
          {m.seasonality_note && <p className="text-[11px] text-neutral-400">{m.seasonality_note}</p>}
        </>
      )}
      {data && (
        <div className="flex flex-col gap-3 pt-2 border-t border-white/10">
          <DriverTable title="Biggest models, last 7 days" rows={data.drivers.models} />
          <DriverTable title="Biggest kinds of work, last 7 days" rows={data.drivers.origins} />
        </div>
      )}
    </Section>
  )
}

const STATUS_TEXT: Record<ProviderRow['status'], string> = {
  ok: 'Reported by the provider',
  refused: 'The provider refused the key',
  error: 'Could not be read',
  no_key: 'No key set',
  local: 'Runs here, nothing billed',
  subscription: 'Subscription quota',
  ledger_only: "HQ's own record only",
}

function ProviderCard({ p }: { p: ProviderRow }) {
  const r = p.provider
  return (
    <Section title={p.backend}>
      <Row label="Source">{p.source === 'provider' ? 'provider' : 'ledger'}, {STATUS_TEXT[p.status]}</Row>
      {r?.balance && <Row label="Balance">{r.balance.amount} {r.balance.currency}</Row>}
      {r?.spend && (
        <Row label="Provider says">{usd(r.spend.today)} today, {usd(r.spend.week)} week, {usd(r.spend.month)} month</Row>
      )}
      {r?.limit_remaining !== null && r?.limit_remaining !== undefined && <Row label="Key limit left">{usd(r.limit_remaining)}</Row>}
      <Row label="HQ recorded">{usd(p.ledger.today)} today, {usd(p.ledger.week)} week, {usd(p.ledger.month)} month</Row>
      {p.ledger.unpriced_calls > 0 && <Row label="Unpriced calls">{p.ledger.unpriced_calls} this month, so the figures are a lower bound</Row>}
      {p.note && <p className="text-[11px] text-neutral-400">{p.note}</p>}
    </Section>
  )
}

function ProvidersPanel() {
  const { data, isLoading } = useQuery(providersQuery)
  return (
    <>
      {isLoading && <div className="flex items-center gap-2 text-xs text-neutral-400"><Loader2 className="w-4 h-4 animate-spin" /> Loading providers</div>}
      {data?.backends.length === 0 && <p className="text-xs text-neutral-400">No backends are configured.</p>}
      {data?.backends.map((p) => <ProviderCard key={p.backend} p={p} />)}
    </>
  )
}

function UsagePage() {
  const qc = useQueryClient()
  const refreshing = qc.isFetching({ queryKey: ['usage-budgets'] }) + qc.isFetching({ queryKey: ['usage-forecast'] }) + qc.isFetching({ queryKey: ['usage-providers'] })
  return (
    <div className="h-full min-h-0 overflow-y-auto overflow-x-hidden p-4 sm:p-6 max-w-5xl w-full mx-auto">
      <div className="flex items-center justify-between mb-4">
        <div>
          <h1 className="text-lg font-semibold text-neutral-100">Usage</h1>
          <p className="text-xs text-neutral-400">What each provider says about its spend, next to what HQ recorded, with budgets and a forecast.</p>
        </div>
        <button
          type="button"
          onClick={() => qc.invalidateQueries({ predicate: (q) => String(q.queryKey[0]).startsWith('usage-') })}
          aria-label="Refresh usage"
          className="p-1.5 rounded-lg text-neutral-400 hover:text-neutral-100 hover:bg-white/5"
        >
          <RefreshCw className={`w-4 h-4 ${refreshing > 0 ? 'animate-spin' : ''}`} />
        </button>
      </div>
      <div className="grid gap-4 md:grid-cols-2">
        <div className="md:col-span-2"><BudgetsPanel /></div>
        <div className="md:col-span-2"><ForecastPanel /></div>
        <ProvidersPanel />
      </div>
    </div>
  )
}
