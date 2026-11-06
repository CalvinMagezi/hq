import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { AlertCircle, MoreHorizontal, RefreshCw, TriangleAlert } from 'lucide-react'
import { toastAlert, toastUndo } from '~/lib/toast'
import { failureLine } from '~/lib/version'
import { useSharedConnection } from '~/lib/connection'
import { spendResetLine } from '~/lib/time'
import { copilotUsageQuery, formatCompact } from '~/lib/copilotUsageApi'
import { t, tn } from '~/strings'
import { LimitSheet } from './LimitSheet'
import { SkeletonRows } from './SettingsSkeleton'
import {
  PERIOD_KEY,
  atLimitWords,
  driverLine,
  scopeWords,
  serviceName,
  spanWords,
  standingWords,
  withLimit,
} from '~/lib/spendLimits'
import {
  budgetsQuery,
  forecastQuery,
  pct,
  providersQuery,
  saveBudgets,
  usd,
  type BudgetDef,
  type BudgetStatus,
  type Driver,
  type ProviderRow,
} from '~/lib/usageApi'

const MAX_BAR_PCT = 100
const SECONDS_TO_MS = 1000
const MS_PER_HOUR = 3_600_000
// The chart palette: blue, sky, white, grey, indigo. Never amber, red or green, which mean something else.
const CHART_COLOURS = ['var(--brand-blue)', 'var(--brand-sky)', 'var(--ink)', 'var(--ink-muted)', 'var(--brand-indigo)']

const hoursUntil = (resetsAt: number) => Math.max(0, (resetsAt * SECONDS_TO_MS - Date.now()) / MS_PER_HOUR)

/** Within the limit is a grey bar; past the warning point and over it are amber. Never green or red. */
function Bar({ value, calm }: { value: number; calm: boolean }) {
  return (
    <div className={`a-prog st-meter ${calm ? 'sp-meter' : 'warn'}`} role="img" aria-label={pct(value)}>
      <u style={{ width: `${Math.min(MAX_BAR_PCT, Math.max(0, value))}%` }} />
    </div>
  )
}

function Fact({ label, children }: { label: string; children: string }) {
  return (
    <div className="st-fact-row a-body-sm">
      <span className="a-muted">{label}</span>
      <span>{children}</span>
    </div>
  )
}

export function LimitRow({ b, def, away, onEdit, onDelete }: { b: BudgetStatus; def: BudgetDef | undefined; away: boolean; onEdit: () => void; onDelete: () => void }) {
  const [menu, setMenu] = useState(false)
  const calm = b.state === 'ok'
  return (
    <li className="a-li st-limit" data-testid="budget-row" data-state={b.state}>
      <div className="st-limit-head">
        <b className="a-body-b">{b.name}</b>
        <span className="a-kind sm">{t(PERIOD_KEY[b.period])}</span>
        <div className="st-menu-anchor">
          <button
            type="button"
            className="a-ib sm"
            aria-label={t('spend_limit_menu', { name: b.name })}
            aria-haspopup="menu"
            aria-expanded={menu}
            disabled={away}
            onClick={() => setMenu(!menu)}
          >
            <MoreHorizontal className="a-ic" aria-hidden="true" />
          </button>
          {menu && (
            <div className="a-menu st-menu" role="menu">
              <button type="button" role="menuitem" className="a-menu-item" onClick={() => { setMenu(false); onEdit() }} disabled={!def}>
                {t('spend_edit')}
              </button>
              <button type="button" role="menuitem" className="a-menu-item danger" onClick={() => { setMenu(false); onDelete() }}>
                {t('spend_delete')}
              </button>
            </div>
          )}
        </div>
      </div>
      <p className="a-body-sm a-muted">{scopeWords(b.scope)}</p>
      <p className="st-limit-spent">
        <span className="a-body-b">{usd(b.spent_usd)}</span>
        <span className="a-body-sm a-muted">
          {t('spend_of', { spent: usd(b.spent_usd), limit: usd(b.limit_usd) })} · {pct(b.pct)}
        </span>
      </p>
      <Bar value={b.pct} calm={calm} />
      <p className={`a-body-sm st-inline ${calm ? 'a-muted' : 'a-warn-t'}`}>
        {!calm && <TriangleAlert className="a-ic" aria-hidden="true" />}
        {standingWords(b, spanWords(hoursUntil(b.resets_at)))}
      </p>
      <p className="a-muted a-body-sm">{atLimitWords({ action: b.action, downgrade_model: def?.downgrade_model })}</p>
    </li>
  )
}

function StoppedBanner({ b, away, onRaise }: { b: BudgetStatus; away: boolean; onRaise: () => void }) {
  return (
    <div className="a-banner" role="alert">
      <TriangleAlert className="a-ic a-warn-t" aria-hidden="true" />
      <div className="min-w-0 flex flex-col gap-2">
        <b>{t('spend_stopped_title')}</b>
        <p>{t('spend_stopped_body', { name: b.name, spent: usd(b.spent_usd), limit: usd(b.limit_usd), span: spanWords(hoursUntil(b.resets_at)) })}</p>
        <button type="button" className="a-btn secondary" disabled={away} onClick={onRaise}>
          {t('spend_raise')}
        </button>
      </div>
    </div>
  )
}

function Loading() {
  return (
    <>
      <p className="a-loading a-muted" role="status">
        {t('spend_loading')}
      </p>
      <SkeletonRows />
    </>
  )
}

function LimitsPanel({ onOpenSheet }: { onOpenSheet: (editing: BudgetDef | null) => void }) {
  const qc = useQueryClient()
  const away = useSharedConnection() !== 'connected'
  const { data, isLoading, isError, refetch } = useQuery(budgetsQuery)
  const save = useMutation({
    mutationFn: (defs: BudgetDef[]) =>
      saveBudgets(defs, { background_run_usd: data?.background_run_usd ?? null, allow_unpriced_models: data?.allow_unpriced_models ?? [] }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['usage-budgets'] }),
  })
  const defs = data?.definitions ?? []
  // Undo puts the whole list back as it was, so the deleted limit returns with every setting.
  const deleteLimit = (name: string) => {
    const before = defs
    const rest = { background_run_usd: data?.background_run_usd ?? null, allow_unpriced_models: data?.allow_unpriced_models ?? [] }
    save.mutate(
      before.filter((d) => d.name !== name),
      {
        onSuccess: () =>
          toastUndo(
            t('done_limit_deleted', { name }),
            async () => {
              await saveBudgets(before, rest)
              await qc.invalidateQueries({ queryKey: ['usage-budgets'] })
            },
            t('restored_limit'),
            'undo.limit_deleted',
          ),
        onError: (err) => toastAlert(failureLine(err, 'fail_delete_limit_newer', t('fail_delete_limit_no_reply'))),
      },
    )
  }
  if (isLoading) return <Loading />
  if (isError || !data) {
    return (
      <div className="a-empty" role="alert">
        <b>{t('spend_load_failed')}</b>
        <p className="a-muted">{t('spend_load_failed_body')}</p>
        <button type="button" className="a-btn secondary" onClick={() => void refetch()}>{t('spend_try_again')}</button>
      </div>
    )
  }
  const stopped = data.budgets.find((b) => b.state === 'exceeded' && b.action === 'block')
  return (
    <>
      {data.problems.map((p) => (
        <p key={p} className="a-warn-t a-body-sm flex items-center gap-1.5" role="alert">
          <AlertCircle className="a-ic" aria-hidden="true" /> {p}
        </p>
      ))}
      {stopped && <StoppedBanner b={stopped} away={away} onRaise={() => onOpenSheet(defs.find((d) => d.name === stopped.name) ?? null)} />}
      {data.budgets.length === 0 ? (
        <div className="a-empty">
          <b>{t('spend_empty_title')}</b>
          <p className="a-muted">{t('spend_empty_body')}</p>
          <button type="button" className="a-btn" disabled={away} onClick={() => onOpenSheet(null)}>{t('spend_add_limit')}</button>
        </div>
      ) : (
        <>
          <h3 className="a-grp">
            {t('spend_limits_group')}
            <span className="a-count">{data.budgets.length}</span>
          </h3>
          <p className="a-body-sm a-muted">{spendResetLine()}</p>
          <ul className="a-set">
            {data.budgets.map((b) => (
              <LimitRow
                key={b.name}
                b={b}
                def={defs.find((d) => d.name === b.name)}
                away={away}
                onEdit={() => onOpenSheet(defs.find((d) => d.name === b.name) ?? null)}
                onDelete={() => deleteLimit(b.name)}
              />
            ))}
          </ul>
        </>
      )}
      {away && <p className="a-muted a-body-sm">{t('spend_needs_hq')}</p>}
    </>
  )
}

function DriverBars({ title, rows }: { title: string; rows: Driver[] }) {
  if (rows.length === 0) return null
  const top = Math.max(...rows.map((d) => d.usd), Number.EPSILON)
  return (
    <div className="a-card">
      <p className="a-label">{title}</p>
      {rows.map((d, i) => (
        <div key={d.name} className="sp-bar">
          <span className="a-body-sm">
            <b>{d.name}</b>
          </span>
          <span className="a-body-sm a-muted">{driverLine(d.calls, usd(d.usd))}</span>
          <span className="a-prog">
            <u style={{ width: `${Math.round((d.usd / top) * MAX_BAR_PCT)}%`, background: CHART_COLOURS[i % CHART_COLOURS.length] }} />
          </span>
        </div>
      ))}
    </div>
  )
}

function ForecastPanel() {
  const { data } = useQuery(forecastQuery)
  const m = data?.month
  if (!data || !m) return null
  const reaching = data.budgets.find((b) => b.forecast.exhausts_before_reset && b.forecast.projected_exhaustion_at)
  return (
    <>
      <section className="a-card flex flex-col gap-2" aria-labelledby="spend-forecast">
        <h2 className="a-h2" id="spend-forecast">{t('spend_forecast_heading')}</h2>
        {m.rate_per_hour === null ? (
          <p className="a-muted">{t('spend_forecast_none')}</p>
        ) : (
          <>
            <Fact label={t('spend_forecast_spent')}>{usd(m.spent_this_period)}</Fact>
            <Fact label={t('spend_forecast_rate')}>{t('spend_forecast_rate_value', { amount: usd(m.rate_per_hour) })}</Fact>
            <Fact label={t('spend_forecast_total')}>{usd(m.projected_period_total)}</Fact>
            <Fact label={t('spend_forecast_resets')}>{spanWords(m.hours_to_reset)}</Fact>
            {reaching && (
              <p className="a-warn-t a-body-sm st-inline">
                <TriangleAlert className="a-ic" aria-hidden="true" />
                {t('spend_forecast_reach', {
                  name: reaching.budget,
                  date: new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short' }).format(new Date(reaching.forecast.projected_exhaustion_at!)),
                })}
              </p>
            )}
            {m.confidence === 'low' && <p className="a-muted a-body-sm">{t('spend_forecast_rough')}</p>}
          </>
        )}
      </section>
      <h3 className="a-grp">{t('spend_used_most')}</h3>
      <div className="sp-two">
        <DriverBars title={t('spend_by_model')} rows={data.drivers.models} />
        <DriverBars title={t('spend_by_work')} rows={data.drivers.origins} />
      </div>
    </>
  )
}

function CopilotCard() {
  const { data } = useQuery(copilotUsageQuery)
  const q = data?.active ? data.quota : undefined
  if (!q) return null
  return (
    <section className="a-card flex flex-col gap-1">
      <h3 className="a-h3">{serviceName('copilot')}</h3>
      <p className="a-muted a-body-sm">{t('spend_svc_plan')}</p>
      <p>{t('spend_svc_credits_left', { left: formatCompact(q.remaining), total: formatCompact(q.entitlement) })}</p>
    </section>
  )
}

const totals = (w: { today: number; week: number; month: number }) =>
  t('spend_svc_service_totals', { today: usd(w.today), week: usd(w.week), month: usd(w.month) })

const outOf = (left: number | null, max: number | null) => (left === null ? null : { left: formatCompact(left), max: max === null ? '' : formatCompact(max) })

function ProviderCard({ p }: { p: ProviderRow }) {
  const name = serviceName(p.backend)
  const r = p.provider
  const requests = p.rate_limit && outOf(p.rate_limit.requests_remaining, p.rate_limit.requests_limit)
  const tokens = p.rate_limit && outOf(p.rate_limit.tokens_remaining, p.rate_limit.tokens_limit)
  return (
    <section className="a-card flex flex-col gap-1" data-testid="service-card">
      <h3 className="a-h3">{name}</h3>
      {p.status === 'refused' && (
        <>
          <p className="a-warn-t flex items-center gap-1.5">
            <AlertCircle className="a-ic" aria-hidden="true" />
            {t('spend_svc_refused')}
          </p>
          <p className="a-muted a-body-sm">{t('spend_svc_refused_body', { name, amount: usd(p.ledger.month) })}</p>
          <a className="a-link" href="/settings/keys">{t('spend_svc_check_key')}</a>
        </>
      )}
      {p.status === 'no_key' && <p className="a-warn-t">{t('spend_svc_no_key')}</p>}
      {p.status === 'error' && <p className="a-warn-t">{t('spend_svc_error')}</p>}
      {p.status === 'local' && <p className="a-muted">{t('spend_svc_local')}</p>}
      {p.status === 'subscription' && <p className="a-muted">{t('spend_svc_plan')}</p>}
      {p.status === 'ledger_only' && <p className="a-muted a-body-sm">{t('spend_svc_no_report', { week: usd(p.ledger.week), month: usd(p.ledger.month) })}</p>}
      {p.status === 'ok' && (
        <>
          <p className="a-muted a-body-sm">{t('spend_svc_says')}</p>
          {r?.balance && <p>{t('spend_svc_balance', { amount: usd(r.balance.amount) })}</p>}
          {r?.limit_remaining !== null && r?.limit_remaining !== undefined && <Fact label={t('spend_svc_key_left')}>{usd(r.limit_remaining)}</Fact>}
          {r?.spend && <p>{totals({ today: r.spend.today ?? 0, week: r.spend.week ?? 0, month: r.spend.month ?? 0 })}</p>}
          <Fact label={t('spend_svc_record')}>{totals(p.ledger)}</Fact>
        </>
      )}
      {(requests || tokens) && (
        <Fact label={t('spend_svc_rate')}>
          {[requests && t('spend_svc_requests', requests), tokens && t('spend_svc_tokens', tokens)].filter(Boolean).join(' · ')}
        </Fact>
      )}
      {p.ledger.unpriced_calls > 0 && <p className="a-muted a-body-sm">{tn('spend_unpriced', p.ledger.unpriced_calls)}</p>}
    </section>
  )
}

function ServicesPanel() {
  const { data, isLoading } = useQuery(providersQuery)
  return (
    <section className="flex flex-col gap-3" aria-labelledby="spend-services">
      <h2 className="a-h2" id="spend-services">{t('spend_services')}</h2>
      {isLoading && <Loading />}
      {data?.backends.length === 0 && <p className="a-muted">{t('spend_no_services')}</p>}
      <div className="sp-two">
        <CopilotCard />
        {data?.backends.map((p) => <ProviderCard key={p.backend} p={p} />)}
      </div>
    </section>
  )
}

/** Spending limits: limits first, then the forecast, what used the most and each AI service. */
export function UsagePage() {
  const qc = useQueryClient()
  const away = useSharedConnection() !== 'connected'
  const budgets = useQuery(budgetsQuery).data
  const providers = useQuery(providersQuery).data
  const forecast = useQuery(forecastQuery).data
  const [sheet, setSheet] = useState<{ editing: BudgetDef | null } | null>(null)
  const refreshing = qc.isFetching({ predicate: (q) => String(q.queryKey[0]).startsWith('usage-') }) > 0
  const save = useMutation({
    mutationFn: (defs: BudgetDef[]) =>
      saveBudgets(defs, { background_run_usd: budgets?.background_run_usd ?? null, allow_unpriced_models: budgets?.allow_unpriced_models ?? [] }),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: ['usage-budgets'] })
      setSheet(null)
    },
  })
  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between gap-2">
        {/* The empty state carries its own Add a limit, so a second one here only repeats it. */}
        {budgets?.definitions.length === 0 ? <span /> : (
          <button type="button" className="a-btn" disabled={away} onClick={() => setSheet({ editing: null })}>
            {t('spend_add_limit')}
          </button>
        )}
        <button
          type="button"
          onClick={() => qc.invalidateQueries({ predicate: (q) => String(q.queryKey[0]).startsWith('usage-') })}
          aria-label={t('spend_refresh')}
          className="a-ib"
        >
          <RefreshCw className={`a-ic ${refreshing ? 'animate-spin' : ''}`} aria-hidden="true" />
        </button>
      </div>
      <LimitsPanel onOpenSheet={(editing) => setSheet({ editing })} />
      <ForecastPanel />
      <ServicesPanel />
      {sheet && (
        <LimitSheet
          editing={sheet.editing}
          existing={budgets?.definitions ?? []}
          services={(providers?.backends ?? []).map((p) => p.backend)}
          models={(forecast?.drivers.models ?? []).map((d) => d.name)}
          busy={save.isPending}
          failed={save.isError}
          resetLine={spendResetLine()}
          onSave={(def) => save.mutate(withLimit(budgets?.definitions ?? [], def, sheet.editing?.name ?? null))}
          onClose={() => {
            save.reset()
            setSheet(null)
          }}
        />
      )}
    </div>
  )
}
