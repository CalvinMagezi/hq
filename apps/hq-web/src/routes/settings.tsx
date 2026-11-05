import { Link, Outlet, createFileRoute, useLocation, useNavigate } from '@tanstack/react-router'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState, type ReactNode } from 'react'
import { ChevronLeft, RefreshCw } from 'lucide-react'
import { fetchSettings, formatSeconds, type HqSettings } from '~/lib/settingsApi'
import { relTime } from '~/lib/time'
import { t, type StringKey } from '~/strings'
import { CardSlot, useBackupStatus, useNotificationsOff } from '~/components/settings/CardSlot'
import { NotificationsPage } from '~/components/settings/NotificationsPage'
import { SettingsHome } from '~/components/settings/SettingsHome'
import { Dialog } from '~/components/ui/Dialog'
import { SignOutDialog } from '~/components/settings/BrowserSections'
import { deviceTimeZone, useClockZone, zoneCity } from '~/lib/clock'
import { readDataUsed, startDataCounter } from '~/lib/dataUsed'
import { fetchHello } from '~/lib/hello'
import { useInHub } from '~/lib/hubWindow'
import { DEFAULT_PUSH_PREFS, readPushPrefs } from '~/lib/webPush'
import { LANDING_PATH, WIDE_MIN_PX, ageWords, backupNeedsLook, formatSize, hashRedirect, liteNotice, type BackupStatus } from '~/lib/settingsGroups'
import { HqHttpError } from '~/lib/hqAuth'
import { ChatAppsSection } from '~/components/settings/ChatAppsSection'
import { formatUsd, openrouterUsageQuery } from '~/lib/openrouterUsageApi'
import { forecastQuery } from '~/lib/usageApi'
import { DevicesSection } from '~/components/settings/DevicesSection'
import { ProviderKeysSection } from '~/components/settings/ProviderKeysSection'
import {
  copilotUsageQuery,
  exhaustionText,
  formatCompact,
  formatHours,
  formatPerHour,
  projectionText,
} from '~/lib/copilotUsageApi'
import {
  SESSION_FOLDERS_QUERY_KEY,
  formatApprovedText,
  listSessionFolders,
  revokeSessionFolder,
  type FolderGrant,
} from '~/lib/sessionFoldersApi'

export const Route = createFileRoute('/settings')({
  component: SettingsLayout,
})

export function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
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
          <Row label="Agent sandbox">
            <Flag on={h.sandbox_mode === 'process'} />
            {h.sandbox_mode === 'process' ? ` · ${h.sandbox_profile ?? 'strict'} profile` : ''}
            {h.sandbox_mode === 'process' && h.sandbox_profile === 'open' && h.sandbox_hide_credentials === false ? ' · credential stores visible' : ''}
            {h.sandbox_profile !== 'open' && h.sandbox_extra_domains ? ` · ${h.sandbox_extra_domains} extra allowed sites` : ''}
          </Row>
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

/** Small text button that runs one action. */
function ActionButton({ busy, onClick, children }: { busy: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" onClick={onClick} disabled={busy} className="hq-btn-ghost disabled:opacity-40">
      {children}
    </button>
  )
}

const HTTP_NOT_FOUND = 404

/** An older server has no folder routes; the section then stays hidden. */
function isMissingRoute(err: unknown): boolean {
  return err instanceof HqHttpError && err.status === HTTP_NOT_FOUND
}

export function SessionFoldersSection() {
  const client = useQueryClient()
  const { data, error } = useQuery({
    queryKey: SESSION_FOLDERS_QUERY_KEY,
    queryFn: listSessionFolders,
    retry: (failureCount, err) => !isMissingRoute(err) && failureCount < 1,
  })
  const act = useMutation({
    mutationFn: (id: number) => revokeSessionFolder(id),
    onSettled: () => client.invalidateQueries({ queryKey: SESSION_FOLDERS_QUERY_KEY }),
  })

  if (isMissingRoute(error)) return null
  if (!data) return null

  const busy = act.isPending
  const [asking, setAsking] = useState<FolderGrant | null>(null)
  const onRevoke = (grant: FolderGrant) => setAsking(grant)

  return (
    <Section id="session-folders" title={t('row_folders')}>
      <p className="py-3.5 text-sm text-neutral-300 leading-relaxed">
        Folders you approved for coding sessions that Claude starts. Each covers the folder and everything under it on that host.
      </p>
      {data.grants.length === 0 ? (
        <p className="py-3.5 text-sm text-neutral-400 border-t border-white/10">
          No folders approved yet. When Claude asks for one, the request appears in your notifications.
        </p>
      ) : (
        data.grants.map((grant) => (
          <Row key={grant.id} label={grant.host}>
            <span className="inline-flex flex-wrap items-baseline justify-end gap-x-3 gap-y-1">
              <span className="font-mono text-xs">{grant.path}</span>
              <span className="text-neutral-400">{formatApprovedText(grant.approved_at, grant.requested_by)}</span>
              <ActionButton busy={busy} onClick={() => onRevoke(grant)}>
                Revoke
              </ActionButton>
            </span>
          </Row>
        ))
      )}
      {asking && (
        <Dialog
          kind="red"
          title={t('revoke_folder_title', { folder: folderName(asking.path) })}
          body={t('revoke_folder_body', { host: asking.host, path: asking.path })}
          keepLabel={t('revoke_folder_keep')}
          actLabel={t('revoke_folder_act')}
          busyLabel={t('confirm_busy_revoking')}
          busy={busy}
          onKeep={() => setAsking(null)}
          onAct={() => {
            act.mutate(asking.id, { onSettled: () => setAsking(null) })
          }}
        />
      )}
      {act.error && (
        <Row label="Error" block>
          {act.error instanceof Error ? act.error.message : 'Could not revoke session folder.'}
        </Row>
      )}
    </Section>
  )
}


/** The settings HQ reports, for the pages that show them; an unreachable HQ shows the line and the saved values. */
/** The last part of a folder path, the way the dialog names it. */
function folderName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}

function useSettingsData() {
  return useQuery({ queryKey: ['settings'], queryFn: fetchSettings })
}

function LoadFailed() {
  return (
    <p className="a-muted a-body-sm" role="status">
      {t('settings_load_failed')}
    </p>
  )
}

export function KeysPage() {
  const { data } = useSettingsData()
  if (!data) return <LoadFailed />
  return (
    <div className="flex flex-col gap-6">
      <ProviderKeysSection rows={data.provider_keys} />
      <ModelSection s={data} />
      <BackendsSection s={data} />
    </div>
  )
}

export function MessagingPage() {
  return <ChatAppsSection />
}

export function DevicesPage() {
  return <DevicesSection />
}

export function SpendingPage() {
  return (
    <div className="flex flex-col gap-6">
      <CopilotCreditsSection />
      <OpenRouterSpendSection />
      <Link to="/usage" className="a-link">
        {t('row_spending')}
      </Link>
    </div>
  )
}

export function SafetyPage() {
  const { data: s } = useSettingsData()
  if (!s) return <LoadFailed />
  return (
    <div className="flex flex-col gap-6">
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

export function FoldersPage() {
  return <SessionFoldersSection />
}

export function BackupPage() {
  const status = useBackupStatus()
  const hello = useQuery({ queryKey: ['hello'], queryFn: ({ signal }) => fetchHello(signal) }).data
  return (
    <div className="flex flex-col gap-4">
      <div className="a-card">
        <b>{t('row_backup')}</b>
        <span className={backupNeedsLook(status) ? 'a-warn-t' : 'a-muted'}>{backupRowWords(status)}</span>
        {hello && <span className="a-muted">{t('ver_hq_line', { version: hello.version })}</span>}
      </div>
    </div>
  )
}

function backupRowWords(status: BackupStatus | null): string {
  return status?.configured && status.last_ok_at ? t('backup_row', { age: ageWords(status.last_ok_at) }) : t('backup_not_reported')
}

const TITLES: Record<string, StringKey> = {
  '/settings/browser': 'row_notifications',
  '/settings/time': 'row_time_zone',
  '/settings/version': 'row_install_version',
  '/settings/data': 'row_data_used',
  '/settings/keys': 'row_ai_keys',
  '/settings/messaging': 'row_messaging',
  '/settings/devices': 'row_other_devices',
  '/settings/spending': 'row_spending',
  '/settings/safety': 'row_safety',
  '/settings/folders': 'row_folders',
  '/settings/backup': 'row_backup',
}

function useWide(): boolean {
  const [wide, setWide] = useState(false)
  useEffect(() => {
    const q = window.matchMedia(`(min-width: ${WIDE_MIN_PX}px)`)
    const sync = () => setWide(q.matches)
    sync()
    q.addEventListener('change', sync)
    return () => q.removeEventListener('change', sync)
  }, [])
  return wide
}

/** Words at the right of each home row, from what this browser already knows. */
function useTrailing(hub: boolean, backup: BackupStatus | null): Record<string, string> {
  const prefs = useStoredPrefs()
  const zone = useClockZone() || deviceTimeZone()
  const [used, setUsed] = useState<number | null>(null)
  useEffect(() => {
    startDataCounter()
    setUsed(readDataUsed())
  }, [])
  const notif = prefs.needs ? (prefs.updates ? 'trail_notif_both' : 'trail_notif_needs') : prefs.updates ? 'trail_notif_updates' : 'trail_notif_none'
  const hello = useQuery({ queryKey: ['hello'], queryFn: ({ signal }) => fetchHello(signal) }).data
  return {
    notifications: hub ? '' : t(notif),
    time: zoneCity(zone),
    version: hello ? t('trail_version', { version: hello.version }) : '',
    data: hub ? t('data_used_row_window') : used === null ? '' : t('data_used_row', { size: formatSize(used) }),
    signout: t(hub ? 'trail_sign_out_window' : 'trail_sign_out_browser'),
    backup: backupRowWords(backup),
  }
}

function useStoredPrefs() {
  const [prefs, setPrefs] = useState(DEFAULT_PUSH_PREFS)
  useEffect(() => setPrefs(readPushPrefs()), [])
  return prefs
}

/** The list pane (the four groups and the card slot) beside the open section; one pane on a phone. */
function SettingsLayout() {
  const { pathname, hash } = useLocation()
  const navigate = useNavigate()
  const wide = useWide()
  const lite = useQuery({ queryKey: ['hello'], queryFn: ({ signal }) => fetchHello(signal) }).data?.edition === 'lite'
  const hub = useInHub()
  const backup = useBackupStatus()
  const notificationsOff = useNotificationsOff()
  const [signingOut, setSigningOut] = useState(false)
  const home = pathname === '/settings' || pathname === '/settings/'

  useEffect(() => {
    const to = hashRedirect(hash)
    if (to) void navigate({ to, replace: true })
    else if (home && wide) void navigate({ to: LANDING_PATH, replace: true })
  }, [hash, home, wide, navigate])

  const trailing = useTrailing(hub, backup)
  const needsLook = [
    ...(backupNeedsLook(backup) ? ['backup'] : []),
    ...(notificationsOff && !wide ? ['notifications'] : []),
  ]
  const notice = liteNotice(pathname, lite)
  const titleKey = notice ? 'row_notifications' : TITLES[pathname]
  const list = (
    <aside className={`min-w-0 lg:w-[360px] lg:shrink-0 lg:overflow-y-auto ${home ? 'block flex-1 lg:flex-none' : 'hidden lg:block'}`}>
      <h1 className="a-h1 px-4 py-3 lg:hidden">{t('settings_title')}</h1>
      <CardSlot notificationsOff={notificationsOff} />
      <SettingsHome lite={lite} hub={hub} trailing={{ ...trailing, notifications: notificationsOff && !wide ? t('trail_notif_none') : trailing.notifications }} needsLook={needsLook} onSignOut={() => setSigningOut(true)} />
    </aside>
  )
  return (
    <div className="h-full min-h-0 flex overflow-hidden">
      {list}
      <main className={`min-w-0 flex-1 overflow-y-auto ${home ? 'hidden lg:block' : 'block'}`}>
        <div className="mx-auto w-full max-w-[760px] px-4 py-5 flex flex-col gap-4">
          {titleKey && (
            <header className="flex items-center gap-2">
              <Link to="/settings" className="a-ib lg:hidden" aria-label={t('settings_back')}>
                <ChevronLeft className="w-4 h-4" />
              </Link>
              <h1 className="a-h1">{t(titleKey)}</h1>
            </header>
          )}
          {notice && (
            <p className="a-body" role="status">
              {t(notice)}
            </p>
          )}
          {notice ? <NotificationsPage /> : <Outlet />}
        </div>
      </main>
      {signingOut && <SignOutDialog onClose={() => setSigningOut(false)} />}
    </div>
  )
}
