import { createFileRoute } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { AlertCircle, Loader2, RefreshCw } from 'lucide-react'
import { fetchSettings, formatSeconds, type HqSettings } from '~/lib/settingsApi'
import { relTime } from '~/lib/time'
import { formatUsd, openrouterUsageQuery } from '~/lib/openrouterUsageApi'
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

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="glass-card rounded-2xl p-4 border border-white/10">
      <h2 className="text-[11px] font-mono uppercase tracking-wider text-neutral-400 mb-3">{title}</h2>
      <dl className="flex flex-col gap-2">{children}</dl>
    </section>
  )
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col sm:flex-row sm:items-baseline sm:justify-between gap-0.5 sm:gap-4 text-xs">
      <dt className="text-neutral-400 flex-shrink-0">{label}</dt>
      <dd className="font-mono text-neutral-100 sm:text-right break-all min-w-0">{children}</dd>
    </div>
  )
}

const yesNo = (value: boolean) => (value ? 'on' : 'off')
const orNone = (value: string | number | null | undefined) => (value === null || value === undefined || value === '' ? 'none' : String(value))

function ModelSection({ s }: { s: HqSettings }) {
  return (
    <Section title="Model">
      <Row label="Active model">{s.model.active}</Row>
      <Row label="Default model">{s.model.default_model}</Row>
      <Row label="Relay override">{orNone(s.model.relay_override)}</Row>
      <Row label="Local providers only">{yesNo(s.model.local_only)}</Row>
    </Section>
  )
}

function BackendsSection({ s }: { s: HqSettings }) {
  const { backends } = s
  if (!backends.configured) {
    return (
      <Section title="Backend chain">
        <Row label="Chain">not configured, using the flat default</Row>
      </Section>
    )
  }
  return (
    <Section title="Backend chain">
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
    <div className="md:col-span-2">
      <Section title="Copilot credits">
        {data.error && <Row label="Status">{data.error}</Row>}
        {data.unavailable && <Row label="Status">Unavailable. {data.note}</Row>}
        {!data.unavailable && data.note && <Row label="Status">{data.note}</Row>}
        {quota && burn && (
          <>
            <div className="flex flex-col gap-1" role="img" aria-label={`${formatCompact(quota.remaining)} of ${formatCompact(total)} credits left`}>
              <div className="flex justify-between text-xs font-mono text-neutral-100">
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
        <button
          type="button"
          onClick={() => refetch()}
          className="self-start mt-1 flex items-center gap-1.5 text-xs text-neutral-400 hover:text-neutral-100"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${isFetching ? 'animate-spin' : ''}`} /> Refresh credits
        </button>
      </Section>
    </div>
  )
}

function OpenRouterSpendSection() {
  const { data, isFetching, refetch } = useQuery(openrouterUsageQuery)
  if (!data?.active) return null
  const u = data.usage
  return (
    <div className="md:col-span-2">
      <Section title="OpenRouter spend">
        {data.error && <Row label="Status">{data.error}</Row>}
        {data.unavailable && <Row label="Status">Unavailable. {data.note}</Row>}
        {u && (
          <>
            <Row label="Today">{formatUsd(u.usage_daily)}</Row>
            <Row label="This week">{formatUsd(u.usage_weekly)}</Row>
            <Row label="This month">{formatUsd(u.usage_monthly)}</Row>
            <Row label="Lifetime, this key">{formatUsd(u.usage)}</Row>
            <Row label="Key spend limit">{u.limit === null ? 'none set' : `${formatUsd(u.limit)} (${formatUsd(u.limit_remaining)} left)`}</Row>
            <Row label="Account credits left">{data.credits_left === null || data.credits_left === undefined ? 'not reported for this key' : formatUsd(data.credits_left)}</Row>
            {u.is_free_tier && <Row label="Plan">free tier</Row>}
            <Row label="Burn rate">not reported by OpenRouter</Row>
            {data.note && <Row label="Note">{data.note}</Row>}
          </>
        )}
        <button
          type="button"
          onClick={() => refetch()}
          className="self-start mt-1 flex items-center gap-1.5 text-xs text-neutral-400 hover:text-neutral-100"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${isFetching ? 'animate-spin' : ''}`} /> Refresh spend
        </button>
      </Section>
    </div>
  )
}

function CodingAgentsSection({ host: h }: { host: HqSettings['agent_host'] }) {
  return (
    <Section title="Coding agents">
      {!h ? (
        <Row label="Settings">not reported by this gateway version, update it to see them</Row>
      ) : (
        <>
          <Row label="Default host">{h.default_host}</Row>
          <Row label="Hosts">{h.hosts.length ? h.hosts.join(', ') : 'this machine only'}</Row>
          <Row label="Agent sandbox">{h.sandbox_mode === 'process' ? 'on' : 'off'}{h.sandbox_extra_domains ? ` · ${h.sandbox_extra_domains} extra allowed sites` : ''}</Row>
          <Row label="Stop idle sessions after">{h.idle_reap_hours ? `${h.idle_reap_hours} h` : 'never'}</Row>
          <Row label="New watches drive">{yesNo(h.drive_new_watches)}</Row>
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
    <div className="grid gap-4 md:grid-cols-2">
      <CopilotCreditsSection />
      <OpenRouterSpendSection />
      <ModelSection s={s} />
      <BackendsSection s={s} />
      <Section title="Provider keys">
        {s.provider_keys.map((k) => (
          <Row key={k.name} label={k.name}>{k.configured ? 'configured' : 'not set'}</Row>
        ))}
      </Section>
      <Section title="Limits">
        <Row label="Chat turn timeout">{formatSeconds(s.limits.chat_turn_timeout_secs)}</Row>
        <Row label="Relay ack window">{formatSeconds(s.limits.turn_ack_timeout_secs)}</Row>
        <Row label="Background turn max age">{s.limits.background_turn_max_days} days</Row>
        <Row label="Background progress every">
          {s.limits.background_progress_secs === null ? 'default' : formatSeconds(s.limits.background_progress_secs)}
        </Row>
      </Section>
      <Section title="Safety">
        <Row label="Bash sandbox">{orNone(s.safety.bash_sandbox)}</Row>
        <Row label="Bash network access">{yesNo(s.safety.bash_network)}</Row>
        <Row label="Skill writes need approval">{yesNo(s.safety.skills_write_approval)}</Row>
        <Row label="Allowed web origins">{s.safety.web_allowed_origins.length ? s.safety.web_allowed_origins.join(', ') : 'none'}</Row>
      </Section>
      <CodingAgentsSection host={s.agent_host} />
      <Section title="Integrations">
        {s.integrations.remote_mcp.length === 0 && <Row label="Remote MCP servers">none</Row>}
        {s.integrations.remote_mcp.map((m) => (
          <Row key={m.name} label={`MCP: ${m.name}`}>{m.host}{m.live_user_turn_only ? ' · live turns only' : ''}</Row>
        ))}
        <Row label="Built-in web search">{s.integrations.web_search_native ? 'on' : 'off'}</Row>
        <Row label="SearxNG">{orNone(s.integrations.searxng_host)}</Row>
        <Row label="Disk watchdog">{yesNo(s.integrations.disk_watchdog_enabled)}</Row>
      </Section>
    </div>
  )
}

function SettingsPage() {
  const { data, error, isLoading, isFetching, refetch } = useQuery({ queryKey: ['settings'], queryFn: fetchSettings })
  return (
    <div className="h-full min-h-0 overflow-y-auto overflow-x-hidden p-4 sm:p-6 max-w-5xl w-full mx-auto">
      <div className="flex items-center justify-between mb-4">
        <div>
          <h1 className="text-lg font-semibold text-neutral-100">Settings</h1>
          <p className="text-xs text-neutral-400">Read-only view of the running HQ configuration. Secrets are never shown.</p>
        </div>
        <button
          type="button"
          onClick={() => refetch()}
          className="p-1.5 rounded-lg text-neutral-400 hover:text-neutral-100 hover:bg-white/5"
          aria-label="Reload settings"
          title="Reload settings"
        >
          <RefreshCw className={`w-4 h-4 ${isFetching ? 'animate-spin' : ''}`} />
        </button>
      </div>
      {isLoading && (
        <div className="flex items-center gap-2 text-xs text-neutral-400"><Loader2 className="w-4 h-4 animate-spin" /> Loading settings</div>
      )}
      {error && !data && (
        <div className="flex items-center gap-2 text-xs text-red-400" role="alert">
          <AlertCircle className="w-4 h-4" /> Could not load settings. {error instanceof Error ? error.message : ''}
        </div>
      )}
      {data && <SettingsBody s={data} />}
    </div>
  )
}
