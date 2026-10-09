import { createFileRoute } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { AlertCircle, Loader2, RefreshCw } from 'lucide-react'
import { fetchSettings, formatSeconds, type HqSettings } from '~/lib/settingsApi'
import { relTime } from '~/lib/time'
import { formatUsd, openrouterUsageQuery } from '~/lib/openrouterUsageApi'
import { forecastQuery } from '~/lib/usageApi'
import {
  copilotUsageQuery,
  exhaustionText,
  formatCompact,
  formatHours,
  formatPerHour,
  projectionText,
} from '~/lib/copilotUsageApi'

export const Route = createFileRoute('/settings')({
  component: SettingsPage,
})

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section id={id} aria-labelledby={`${id}-title`} className="scroll-mt-4">
      <h2 id={`${id}-title`} className="mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400">
        {title}
      </h2>
      <dl className="hq-card rounded-2xl px-5">{children}</dl>
    </section>
  )
}

/** One setting: its name on the left and its value on the right. `block` stacks long prose under the name instead. */
function Row({ label, children, block }: { label: string; children: ReactNode; block?: boolean }) {
  return (
    <div className={`flex gap-4 py-3.5 text-sm border-t border-white/10 first:border-t-0 ${block ? 'flex-col gap-1' : 'items-baseline justify-between'}`}>
      <dt className="text-neutral-300 shrink-0 max-w-[50%]">{label}</dt>
      <dd className={`min-w-0 break-words text-neutral-50 ${block ? 'text-neutral-300 leading-relaxed' : 'text-right'}`}>{children}</dd>
    </div>
  )
}

/** On or off as a word with a dot, so the state never rests on colour alone. */
function Flag({ on, onLabel = 'On', offLabel = 'Off' }: { on: boolean; onLabel?: string; offLabel?: string }) {
  return (
    <span className={`inline-flex items-center gap-2 ${on ? 'text-emerald-300' : 'text-neutral-400'}`}>
      <span className={`w-2 h-2 rounded-full ${on ? 'bg-emerald-400' : 'bg-neutral-500'}`} aria-hidden="true" />
      {on ? onLabel : offLabel}
    </span>
  )
}

function RefreshLink({ busy, onClick, children }: { busy: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <div className="py-3 border-t border-white/10">
      <button type="button" onClick={onClick} className="hq-btn-ghost">
        <RefreshCw className={`w-3.5 h-3.5 ${busy ? 'animate-spin' : ''}`} /> {children}
      </button>
    </div>
  )
}

const orNone = (value: string | number | null | undefined) => (value === null || value === undefined || value === '' ? 'none' : String(value))

function ModelSection({ s }: { s: HqSettings }) {
  return (
    <Section id="model" title="Model">
      <Row label="Active model">{s.model.active}</Row>
      <Row label="Default model">{s.model.default_model}</Row>
      <Row label="Relay override">{orNone(s.model.relay_override)}</Row>
      <Row label="Local providers only"><Flag on={s.model.local_only} /></Row>
    </Section>
  )
}

function BackendsSection({ s }: { s: HqSettings }) {
  const { backends } = s
  if (!backends.configured) {
    return (
      <Section id="backends" title="Backend chain">
        <Row label="Chain">not configured, using the flat default</Row>
      </Section>
    )
  }
  return (
    <Section id="backends" title="Backend chain">
      {backends.entries.map((b) => (
        <Row key={b.name} label={b.primary ? `${b.name} (primary)` : b.fallback_position === null ? b.name : `${b.name} (fallback ${b.fallback_position + 1})`}>
          {[orNone(b.kind), orNone(b.model), b.effort, b.endpoint_host].filter(Boolean).join(' · ')}
        </Row>
      ))}
    </Section>
  )
}

const PERCENT_MAX = 100

function CopilotCreditsSection() {
  const { data, isFetching, refetch } = useQuery(copilotUsageQuery)
  if (!data?.active) return null
  const { quota, burn } = data
  const total = quota?.entitlement ?? 0
  const leftPct = quota && total > 0 ? Math.min(PERCENT_MAX, Math.max(0, (quota.remaining / total) * PERCENT_MAX)) : 0
  return (
    <>
      <Section id="credits" title="Copilot credits">
        {data.error && <Row label="Status" block>{data.error}</Row>}
        {data.unavailable && <Row label="Status" block>Unavailable. {data.note}</Row>}
        {!data.unavailable && data.note && <Row label="Status" block>{data.note}</Row>}
        {quota && burn && (
          <>
            <div className="flex flex-col gap-1" role="img" aria-label={`${formatCompact(quota.remaining)} of ${formatCompact(total)} credits left`}>
              <div className="flex justify-between text-xs text-neutral-100">
                <span>{formatCompact(quota.remaining)} left of {formatCompact(total)}</span>
                <span>{Math.round(leftPct)}%</span>
              </div>
              <div className="h-2 rounded-full overflow-hidden" style={{ background: 'var(--border)' }}>
                <div className="h-full rounded-full" style={{ width: `${leftPct}%`, background: 'var(--accent-blue)' }} />
              </div>
            </div>
            <Row label="Used">{formatCompact(quota.credits_used)}</Row>
            {burn.windows.map((w) => (
              <Row key={w.window_hours} label={`Burn, last ${w.window_hours}h`}>{formatPerHour(w.per_hour)}</Row>
            ))}
            <Row label="Projected use at reset">{projectionText(burn, total)}</Row>
            <Row label="Time to reset">{formatHours(burn.hours_to_reset)}</Row>
            <Row label="Runs out">{exhaustionText(burn)}</Row>
            <Row label="Account">{[quota.login, quota.plan].filter(Boolean).join(' · ') || 'unknown'}</Row>
            <Row label="Samples">{burn.sample_count} ({burn.confidence} confidence), read {relTime(quota.fetched_at)}</Row>
          </>
        )}
        <RefreshLink busy={isFetching} onClick={() => refetch()}>Refresh credits</RefreshLink>
      </Section>
    </>
  )
}

function OpenRouterSpendSection() {
  const { data, isFetching, refetch } = useQuery(openrouterUsageQuery)
  const forecast = useQuery(forecastQuery).data?.month
  if (!data?.active) return null
  const u = data.usage
  return (
    <>
      <Section id="spend" title="OpenRouter spend">
        {data.error && <Row label="Status" block>{data.error}</Row>}
        {data.unavailable && <Row label="Status" block>Unavailable. {data.note}</Row>}
        {u && (
          <>
            <Row label="Today">{formatUsd(u.usage_daily)}</Row>
            <Row label="This week">{formatUsd(u.usage_weekly)}</Row>
            <Row label="This month">{formatUsd(u.usage_monthly)}</Row>
            <Row label="Lifetime, this key">{formatUsd(u.usage)}</Row>
            <Row label="Key spend limit">{u.limit === null ? 'none set' : `${formatUsd(u.limit)} (${formatUsd(u.limit_remaining)} left)`}</Row>
            <Row label="Account credits left">{data.credits_left === null || data.credits_left === undefined ? 'not reported for this key' : formatUsd(data.credits_left)}</Row>
            {u.is_free_tier && <Row label="Plan">free tier</Row>}
            <Row label="Burn rate">
              {forecast?.rate_per_hour == null
                ? 'not enough history yet'
                : `${formatUsd(forecast.rate_per_hour)} per hour (HQ's estimate, ${forecast.confidence} confidence)`}
            </Row>
            {data.note && <Row label="Note" block>{data.note}</Row>}
          </>
        )}
        <RefreshLink busy={isFetching} onClick={() => refetch()}>Refresh spend</RefreshLink>
      </Section>
    </>
  )
}

function CodingAgentsSection({ host: h }: { host: HqSettings['agent_host'] }) {
  return (
    <Section id="agents" title="Coding agents">
      {!h ? (
        <Row label="Settings">not reported by this gateway version, update it to see them</Row>
      ) : (
        <>
          <Row label="Default host">{h.default_host}</Row>
          <Row label="Hosts">{h.hosts.length ? h.hosts.join(', ') : 'this machine only'}</Row>
          <Row label="Agent sandbox"><Flag on={h.sandbox_mode === 'process'} />{h.sandbox_extra_domains ? ` · ${h.sandbox_extra_domains} extra allowed sites` : ''}</Row>
          <Row label="Stop idle sessions after">{h.idle_reap_hours ? `${h.idle_reap_hours} h` : 'never'}</Row>
          <Row label="New watches drive"><Flag on={h.drive_new_watches} /></Row>
          <Row label="Driver check-in">{h.driver_checkin_minutes} min</Row>
          <Row label="Driver instruction budget">{h.driver_nudge_budget} per session</Row>
          <Row label="Stop after idle turns">{h.driver_no_progress_limit} with no tool activity</Row>
        </>
      )}
    </Section>
  )
}

function SettingsBody({ s }: { s: HqSettings }) {
  return (
    <div className="flex flex-col gap-6 min-w-0 flex-1 max-w-[760px]">
      <CopilotCreditsSection />
      <OpenRouterSpendSection />
      <ModelSection s={s} />
      <BackendsSection s={s} />
      <Section id="keys" title="Provider keys">
        {s.provider_keys.map((k) => (
          <Row key={k.name} label={k.name}><Flag on={k.configured} onLabel="Configured" offLabel="Not set" /></Row>
        ))}
      </Section>
      <Section id="limits" title="Limits">
        <Row label="Chat turn timeout">{formatSeconds(s.limits.chat_turn_timeout_secs)}</Row>
        <Row label="Relay ack window">{formatSeconds(s.limits.turn_ack_timeout_secs)}</Row>
        <Row label="Background turn max age">{s.limits.background_turn_max_days} days</Row>
        <Row label="Background progress every">
          {s.limits.background_progress_secs === null ? 'default' : formatSeconds(s.limits.background_progress_secs)}
        </Row>
      </Section>
      <Section id="safety" title="Safety">
        <Row label="Bash sandbox">{orNone(s.safety.bash_sandbox)}</Row>
        <Row label="Bash network access"><Flag on={s.safety.bash_network} /></Row>
        <Row label="Skill writes need approval"><Flag on={s.safety.skills_write_approval} /></Row>
        <Row label="Allowed web origins">{s.safety.web_allowed_origins.length ? s.safety.web_allowed_origins.join(', ') : 'none'}</Row>
      </Section>
      <CodingAgentsSection host={s.agent_host} />
      <Section id="integrations" title="Integrations">
        {s.integrations.remote_mcp.length === 0 && <Row label="Remote MCP servers">none</Row>}
        {s.integrations.remote_mcp.map((m) => (
          <Row key={m.name} label={`MCP: ${m.name}`}>{m.host}{m.live_user_turn_only ? ' · live turns only' : ''}</Row>
        ))}
        <Row label="Built-in web search"><Flag on={s.integrations.web_search_native} /></Row>
        <Row label="SearxNG">{orNone(s.integrations.searxng_host)}</Row>
        <Row label="Disk watchdog"><Flag on={s.integrations.disk_watchdog_enabled} /></Row>
      </Section>
    </div>
  )
}

const NAV = [
  ['model', 'Model'],
  ['backends', 'Backend chain'],
  ['keys', 'Provider keys'],
  ['limits', 'Limits'],
  ['safety', 'Safety'],
  ['agents', 'Coding agents'],
  ['integrations', 'Integrations'],
] as const

function SettingsPage() {
  const { data, error, isLoading, isFetching, refetch } = useQuery({ queryKey: ['settings'], queryFn: fetchSettings })
  return (
    <div className="h-full min-h-0 overflow-y-auto overflow-x-hidden">
      <div className="mx-auto w-full max-w-5xl px-4 py-5 sm:px-8 sm:py-8">
        <header className="flex items-start justify-between gap-4 mb-6">
          <div>
            <h1 className="text-3xl font-semibold tracking-tight text-neutral-50" style={{ fontFamily: 'var(--font-heading)' }}>Settings</h1>
            <p className="mt-1 text-sm text-neutral-400 max-w-prose">Read-only view of the running HQ configuration. Secrets are never shown.</p>
          </div>
          <button type="button" onClick={() => refetch()} className="hq-icon-btn" aria-label="Reload settings" title="Reload settings">
            <RefreshCw className={`w-4 h-4 ${isFetching ? 'animate-spin' : ''}`} />
          </button>
        </header>
        {isLoading && (
          <div className="flex items-center gap-2 text-sm text-neutral-400"><Loader2 className="w-4 h-4 animate-spin" /> Loading settings</div>
        )}
        {error && !data && (
          <div className="flex items-center gap-2 text-sm text-red-400" role="alert">
            <AlertCircle className="w-4 h-4" /> Could not load settings. {error instanceof Error ? error.message : ''}
          </div>
        )}
        {data && (
          <div className="flex gap-10 items-start">
            <nav aria-label="Settings sections" className="hidden lg:flex flex-col gap-1 w-48 shrink-0 sticky top-2">
              {NAV.map(([id, label]) => (
                <a key={id} href={`#${id}`} className="px-3 py-2 rounded-xl text-sm text-neutral-300 hover:bg-white/5 hover:text-white">
                  {label}
                </a>
              ))}
            </nav>
            <SettingsBody s={data} />
          </div>
        )}
      </div>
    </div>
  )
}
