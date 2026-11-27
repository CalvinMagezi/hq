import { Link, Outlet, createFileRoute, useLocation, useNavigate } from '@tanstack/react-router'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { TrustedSites } from '~/components/settings/TrustedSites'
import { HelpersAsk } from '~/components/settings/HelpersAsk'
import { useEffect, useState } from 'react'
import { ChevronLeft } from 'lucide-react'
import { fetchSettings, type HqSettings } from '~/lib/settingsApi'
import { t, type StringKey } from '~/strings'
import { useSharedConnection } from '~/lib/connection'
import { BackupView, FoldersView, SafetyView, type SafetyTab } from '~/components/settings/SettingsViews'
import { CardSlot, useBackupStatus, useNotificationsOff } from '~/components/settings/CardSlot'
import { NotificationsPage } from '~/components/settings/NotificationsPage'
import { SettingsHome } from '~/components/settings/SettingsHome'
import { SettingsFind } from '~/components/settings/SettingsFind'
import { readEnterSends } from '~/lib/enterRule'
import { useHQStore } from '~/store/hqStore'
import { globalSessionsApi } from '~/lib/sessionsApi'
import { helpersLine, tasksLine } from '~/lib/workLines'
import { readPaneMode } from '~/lib/paneMode'
import { Dialog } from '~/components/ui/Dialog'
import { PaneHead, Panes } from '~/components/ui/Panes'
import { SettingsSkeleton } from '~/components/settings/SettingsSkeleton'
import { SignOutDialog } from '~/components/settings/BrowserSections'
import { deviceTimeZone, useClockZone, zoneCity } from '~/lib/clock'
import { spendResetLine, useHqZone, zonesDiffer } from '~/lib/time'
import { readDataUsed } from '~/lib/dataUsed'
import { startDataCounter } from '~/lib/dataUsage'
import { fetchHello } from '~/lib/hello'
import { useHubPhone, useInHub } from '~/lib/hubWindow'
import { DEFAULT_PUSH_PREFS, getPushState, readPushPrefs } from '~/lib/webPush'
import { LANDING_PATH, WIDE_MIN_PX, ageWords, trailNotifKey, githubTrail, backupNeedsLook, formatSize, hashRedirect, liteNotice, type BackupStatus, type GithubTrail } from '~/lib/settingsGroups'
import { HqHttpError } from '~/lib/hqAuth'
import { ChatAppsSection } from '~/components/settings/ChatAppsSection'
import { UsagePage } from '~/components/settings/UsageLimits'
import { DevicesSection } from '~/components/settings/DevicesSection'
import { GithubConnect } from '~/components/sessions/GithubConnect'
import { GITHUB_QUERY_KEY, githubApi } from '~/lib/github'
import { ProviderKeysSection } from '~/components/settings/ProviderKeysSection'
import {
  SESSION_FOLDERS_QUERY_KEY,
  listSessionFolders,
  revokeSessionFolder,
  type FolderGrant,
} from '~/lib/sessionFoldersApi'
import { AccessView } from '~/components/settings/AccessView'
import { MemorySection } from '~/components/settings/MemorySection'
import {
  SANDBOX_GRANTS_QUERY_KEY,
  SANDBOX_PROFILES_QUERY_KEY,
  SANDBOX_RULES_QUERY_KEY,
  SANDBOX_SUMMARY_QUERY_KEY,
  getSandboxSummary,
  listSandboxGrants,
  listSandboxProfiles,
  listSandboxRules,
  revokeSandboxGrant,
} from '~/lib/sandboxGrantsApi'

export const Route = createFileRoute('/settings')({
  component: SettingsLayout,
  notFoundComponent: SettingsNotHere,
})

function SettingsNotHere() {
  return (
    <div role="status" className="flex flex-col gap-2">
      <h1 className="a-h1">{t('settings_not_here_title')}</h1>
      <p className="a-body">{t('settings_not_here_body')}</p>
    </div>
  )
}

function useSettingsData() {
  return useQuery({ queryKey: ['settings'], queryFn: fetchSettings })
}

const HTTP_NOT_FOUND = 404

/** An older server has no folder routes; the section then stays hidden. */
function isMissingRoute(err: unknown): boolean {
  return err instanceof HqHttpError && err.status === HTTP_NOT_FOUND
}

/** The last part of a folder path, the way the dialog names it. */
function folderName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}

function LoadFailed({ words = 'settings_failed_safety', onRetry }: { words?: StringKey; onRetry?: () => void }) {
  return (
    <div className="a-card" role="status">
      <b>{t(words)}</b>
      <span className="a-muted">{t('settings_failed_body')}</span>
      {onRetry && (
        <button type="button" className="a-btn secondary" onClick={onRetry}>
          {t('settings_try_again')}
        </button>
      )}
    </div>
  )
}

export function SessionFoldersSection() {
  const client = useQueryClient()
  const reachable = useSharedConnection() === 'connected'
  const { data, error } = useQuery({
    queryKey: SESSION_FOLDERS_QUERY_KEY,
    queryFn: listSessionFolders,
    retry: (failureCount, err) => !isMissingRoute(err) && failureCount < 1,
  })
  const act = useMutation({
    mutationFn: (id: number) => revokeSessionFolder(id),
    onSettled: () => client.invalidateQueries({ queryKey: SESSION_FOLDERS_QUERY_KEY }),
  })
  const [asking, setAsking] = useState<FolderGrant | null>(null)

  if (isMissingRoute(error)) return null
  if (!data) {
    return error ? <LoadFailed onRetry={() => client.invalidateQueries({ queryKey: SESSION_FOLDERS_QUERY_KEY })} /> : <SettingsSkeleton label={t('settings_loading')} />
  }

  return (
    <>
      <FoldersView grants={data.grants} canChange={reachable} busy={act.isPending} onRevoke={setAsking} />
      {asking && (
        <Dialog
          kind="red"
          title={t('revoke_folder_title', { folder: folderName(asking.path) })}
          body={t('revoke_folder_body', { host: asking.host, path: asking.path })}
          keepLabel={t('revoke_folder_keep')}
          actLabel={t('revoke_folder_act')}
          busyLabel={t('confirm_busy_revoking')}
          busy={act.isPending}
          onKeep={() => setAsking(null)}
          onAct={() => {
            act.mutate(asking.id, { onSettled: () => setAsking(null) })
          }}
        />
      )}
    </>
  )
}

function WhichAi({ s }: { s: HqSettings }) {
  const { backends, model } = s
  return (
    <section className="flex flex-col gap-2">
      <h3 className="a-grp">{t('settings_which_ai')}</h3>
      <p className="a-muted a-body-sm">{t('settings_ai_cannot_change')}</p>
      <div className="a-set">
        <div className="a-li">
          <div className="a-li-main">
            <b>{t('settings_ai_model')}</b>
            <span>{model.active}</span>
            {model.local_only && <span>{t('settings_ai_local_only')}</span>}
          </div>
        </div>
      </div>
      {backends.configured && (
        <>
          <h3 className="a-grp">{t('settings_which_answers')}</h3>
          <div className="a-set">
            {backends.entries.map((b) => (
              <div key={b.name} className="a-li">
                <div className="a-li-main">
                  <b>{b.name}</b>
                  <span>
                    {[b.model, b.effort ? t('settings_chain_effort', { effort: b.effort }) : null, b.endpoint_host ? t('settings_chain_at', { host: b.endpoint_host }) : null]
                      .filter(Boolean)
                      .join(' · ')}
                  </span>
                  {!b.primary && <span>{t('settings_chain_next')}</span>}
                </div>
                {b.primary && <span className="a-kind sm">{t('settings_chain_first')}</span>}
              </div>
            ))}
          </div>
        </>
      )}
    </section>
  )
}

export function KeysPage() {
  const { data, isError, refetch } = useSettingsData()
  if (!data) return isError ? <LoadFailed words="settings_failed_safety" onRetry={() => void refetch()} /> : <SettingsSkeleton label={t('settings_loading')} />
  return (
    <div className="flex flex-col gap-4">
      <ProviderKeysSection rows={data.provider_keys} />
      <WhichAi s={data} />
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
  return <UsagePage />
}

export function SafetyPage() {
  const { data: s, isError, refetch } = useSettingsData()
  const [tab, setTab] = useState<SafetyTab>('safety')
  if (!s) return isError ? <LoadFailed onRetry={() => void refetch()} /> : <SettingsSkeleton label={t('settings_loading')} />
  return (
    <>
      <SafetyView s={s} tab={tab} onTab={setTab} />
      {tab === 'safety' && <HelpersAsk />}
      {tab === 'safety' && <TrustedSites />}
    </>
  )
}

export function GithubPage() {
  return <GithubConnect />
}

export function FoldersPage() {
  return <SessionFoldersSection />
}

const retryUnlessMissing = (failureCount: number, err: unknown) => !isMissingRoute(err) && failureCount === 0

export function AccessSection() {
  const client = useQueryClient()
  const reachable = useSharedConnection() === 'connected'
  const grants = useQuery({
    queryKey: SANDBOX_GRANTS_QUERY_KEY,
    queryFn: () => listSandboxGrants(),
    retry: retryUnlessMissing,
  })
  const profiles = useQuery({
    queryKey: SANDBOX_PROFILES_QUERY_KEY,
    queryFn: listSandboxProfiles,
    retry: retryUnlessMissing,
  })
  const rules = useQuery({
    queryKey: SANDBOX_RULES_QUERY_KEY,
    queryFn: listSandboxRules,
    retry: retryUnlessMissing,
  })
  const summary = useQuery({
    queryKey: SANDBOX_SUMMARY_QUERY_KEY,
    queryFn: () => getSandboxSummary(7),
    retry: retryUnlessMissing,
  })
  const [revokeError, setRevokeError] = useState<string | null>(null)
  const act = useMutation({
    mutationFn: (id: string) => revokeSandboxGrant(id),
    onMutate: () => setRevokeError(null),
    onError: (e) => setRevokeError(e instanceof Error ? e.message : t('grants_revoke_failed')),
    onSettled: () => {
      void client.invalidateQueries({ queryKey: SANDBOX_GRANTS_QUERY_KEY })
      void client.invalidateQueries({ queryKey: SANDBOX_RULES_QUERY_KEY })
      void client.invalidateQueries({ queryKey: SANDBOX_SUMMARY_QUERY_KEY })
    },
  })

  if (isMissingRoute(grants.error) && isMissingRoute(profiles.error)) return null
  if (!grants.data || !profiles.data) {
    return grants.error || profiles.error ? (
      <LoadFailed
        onRetry={() => {
          void client.invalidateQueries({ queryKey: SANDBOX_GRANTS_QUERY_KEY })
          void client.invalidateQueries({ queryKey: SANDBOX_PROFILES_QUERY_KEY })
          void client.invalidateQueries({ queryKey: SANDBOX_RULES_QUERY_KEY })
          void client.invalidateQueries({ queryKey: SANDBOX_SUMMARY_QUERY_KEY })
        }}
      />
    ) : (
      <SettingsSkeleton label={t('settings_loading')} />
    )
  }

  return (
    <AccessView
      grants={grants.data.grants}
      profiles={profiles.data.profiles}
      rules={rules.data?.rules ?? []}
      summary={summary.data ?? null}
      canChange={reachable}
      revokingId={act.isPending ? (act.variables ?? null) : null}
      error={revokeError}
      onRevoke={(id) => act.mutate(id)}
    />
  )
}

export function AccessPage() {
  return <AccessSection />
}

export function MemoryPage() {
  return <MemorySection />
}

export function BackupPage() {
  const status = useBackupStatus()
  const hello = useQuery({ queryKey: ['hello'], queryFn: ({ signal }) => fetchHello(signal) }).data
  return <BackupView status={status} canStart={false} version={hello?.version ?? null} />
}

const PANE_WORD = { auto: 'chat_set_pane_auto', card: 'chat_set_pane_card', never: 'chat_set_pane_never' } as const

function backupRowWords(status: BackupStatus | null): string {
  return status?.configured && status.last_ok_at ? t('backup_row', { age: ageWords(status.last_ok_at) }) : t('backup_not_reported')
}

const TITLES: Record<string, StringKey> = {
  '/settings/chat': 'row_chat',
  '/settings/browser': 'row_notifications',
  '/settings/time': 'row_time_zone',
  '/settings/version': 'row_install_version',
  '/settings/data': 'row_data_used',
  '/settings/keys': 'row_ai_keys',
  '/settings/messaging': 'row_messaging',
  '/settings/devices': 'row_other_devices',
  '/settings/github': 'row_github',
  '/settings/spending': 'row_spending',
  '/settings/safety': 'row_safety',
  '/settings/folders': 'row_folders',
  '/settings/access': 'row_access',
  '/settings/memory': 'row_memory',
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
function useTrailing(hub: boolean, phone: boolean, backup: BackupStatus | null, github: GithubTrail | null): Record<string, string> {
  const prefs = useStoredPrefs()
  const pushOn = usePushOn()
  const zone = useClockZone() || deviceTimeZone()
  const hqZone = useHqZone()
  const [used, setUsed] = useState<number | null>(null)
  useEffect(() => {
    startDataCounter()
    setUsed(readDataUsed())
  }, [])
  const notif = trailNotifKey(prefs, pushOn)
  const hello = useQuery({ queryKey: ['hello'], queryFn: ({ signal }) => fetchHello(signal) }).data
  const tasks = useHQStore((s) => s.tasks)
  const helpers = useQuery({ queryKey: ['search', 'helpers'], queryFn: () => globalSessionsApi.list({}), staleTime: 30_000, retry: false }).data ?? []
  return {
    notifications: hub ? '' : t(notif),
    time: !hqZone ? zoneCity(zone) : zonesDiffer(hqZone, zone) ? t('tz_row_differ', { city: zoneCity(zone), hqCity: zoneCity(hqZone) }) : t('tz_row_same', { city: zoneCity(zone) }),
    version: hello ? t('trail_version', { version: hello.version }) : '',
    data: phone ? t('data_used_row_window') : used === null ? '' : t('data_used_row', { size: formatSize(used) }),
    signout: t(hub ? 'trail_sign_out_window' : 'trail_sign_out_browser'),
    backup: backupRowWords(backup),
    github: github ? t(github.key, { owner: github.owner ?? '' }) : '',
    chat: t('trail_chat', {
      enter: t(readEnterSends() ?? !window.matchMedia('(pointer: coarse)').matches ? 'chat_set_enter_on' : 'chat_set_enter_off'),
      pane: t(PANE_WORD[readPaneMode()]),
    }),
    work_tasks: tasksLine(tasks),
    work_helpers: helpersLine(helpers),
  }
}

function usePushOn(): boolean {
  const [on, setOn] = useState(false)
  useEffect(() => {
    getPushState()
      .then((s) => setOn(s === 'on'))
      .catch(() => undefined)
  }, [])
  return on
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
  const phone = useHubPhone()
  const backup = useBackupStatus()
  const notificationsOff = useNotificationsOff()
  const [signingOut, setSigningOut] = useState(false)
  const home = pathname === '/settings' || pathname === '/settings/'

  useEffect(() => {
    const to = hashRedirect(hash)
    if (to) void navigate({ to, replace: true })
    else if (home && wide) void navigate({ to: LANDING_PATH, replace: true })
  }, [hash, home, wide, navigate])

  // Lite has no HQ Coder, so it is never asked.
  const github = githubTrail(useQuery({ queryKey: GITHUB_QUERY_KEY, queryFn: githubApi.status, enabled: !lite, retry: false }).data)
  const trailing = useTrailing(hub, phone, backup, github)
  const needsLook = [
    ...(github?.needsLook ? ['github'] : []),
    ...(backupNeedsLook(backup) ? ['backup'] : []),
    ...(notificationsOff && !wide ? ['notifications'] : []),
  ]
  const notice = liteNotice(pathname, lite)
  const titleKey = notice ? 'row_notifications' : TITLES[pathname]
  const list = (
    <>
      <SettingsFind lite={lite} hub={hub} onSignOut={() => setSigningOut(true)}>
        <CardSlot notificationsOff={notificationsOff} />
        <SettingsHome lite={lite} hub={hub} phone={phone} trailing={{ ...trailing, notifications: notificationsOff && !wide ? t('trail_notif_none') : trailing.notifications }} needsLook={needsLook} onSignOut={() => setSigningOut(true)} />
      </SettingsFind>
    </>
  )
  const detail = (
    <>
      {titleKey && (
        <PaneHead
          title={t(titleKey)}
          back={
            <Link to="/settings" className="a-ib a-back" aria-label={t('settings_back')}>
              <ChevronLeft className="a-ic" aria-hidden="true" />
            </Link>
          }
        />
      )}
      <div className="st-body">
        <div className="a-wrap">
          {notice && (
            <p className="a-body" role="status">
              {t(notice)}
            </p>
          )}
          {notice ? <NotificationsPage /> : <Outlet />}
        </div>
      </div>
    </>
  )
  return (
    <>
      <Panes list={list} detail={detail} showDetail={!home} listLabel={t('settings_title')} detailLabel={titleKey ? t(titleKey) : t('settings_title')} />
      {signingOut && <SignOutDialog onClose={() => setSigningOut(false)} />}
    </>
  )
}
