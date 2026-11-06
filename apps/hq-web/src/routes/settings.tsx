import { Link, Outlet, createFileRoute, useLocation, useNavigate } from '@tanstack/react-router'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState, type ReactNode } from 'react'
import { ChevronLeft } from 'lucide-react'
import { fetchSettings, type HqSettings } from '~/lib/settingsApi'
import { t, type StringKey } from '~/strings'
import { useSharedConnection } from '~/lib/connection'
import { BackupView, FoldersView, SafetyView, type SafetyTab } from '~/components/settings/SettingsViews'
import { CardSlot, useBackupStatus, useNotificationsOff } from '~/components/settings/CardSlot'
import { NotificationsPage } from '~/components/settings/NotificationsPage'
import { SettingsHome } from '~/components/settings/SettingsHome'
import { Dialog } from '~/components/ui/Dialog'
import { SignOutDialog } from '~/components/settings/BrowserSections'
import { deviceTimeZone, useClockZone, zoneCity } from '~/lib/clock'
import { spendResetLine, useHqZone, zonesDiffer } from '~/lib/time'
import { readDataUsed } from '~/lib/dataUsed'
import { startDataCounter } from '~/lib/dataUsage'
import { fetchHello } from '~/lib/hello'
import { useHubPhone, useInHub } from '~/lib/hubWindow'
import { DEFAULT_PUSH_PREFS, getPushState, readPushPrefs } from '~/lib/webPush'
import { LANDING_PATH, WIDE_MIN_PX, ageWords, trailNotifKey, backupNeedsLook, formatSize, hashRedirect, liteNotice, type BackupStatus } from '~/lib/settingsGroups'
import { HqHttpError } from '~/lib/hqAuth'
import { ChatAppsSection } from '~/components/settings/ChatAppsSection'
import { UsagePage } from '~/components/settings/UsageLimits'
import { DevicesSection } from '~/components/settings/DevicesSection'
import { ProviderKeysSection } from '~/components/settings/ProviderKeysSection'
import {
  SESSION_FOLDERS_QUERY_KEY,
  listSessionFolders,
  revokeSessionFolder,
  type FolderGrant,
} from '~/lib/sessionFoldersApi'

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
  if (!data) return error ? <LoadFailed onRetry={() => client.invalidateQueries({ queryKey: SESSION_FOLDERS_QUERY_KEY })} /> : null

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
      <h2 className="a-h3">{t('settings_which_ai')}</h2>
      <p className="a-muted a-body-sm">{t('settings_ai_cannot_change')}</p>
      <div className="a-card">
        <b>{t('settings_ai_model')}</b>
        <span>{model.active}</span>
        {model.local_only && <span className="a-muted">{t('settings_ai_local_only')}</span>}
      </div>
      {backends.configured && (
        <>
          <h3 className="a-h3">{t('settings_which_answers')}</h3>
          {backends.entries.map((b) => (
            <div key={b.name} className="a-card">
              <b>{b.name}</b>
              <span className="a-muted">
                {[b.model, b.effort ? t('settings_chain_effort', { effort: b.effort }) : null, b.endpoint_host ? t('settings_chain_at', { host: b.endpoint_host }) : null]
                  .filter(Boolean)
                  .join(' · ')}
              </span>
              <span className="a-muted a-body-sm">{b.primary ? t('settings_chain_first') : t('settings_chain_next')}</span>
            </div>
          ))}
        </>
      )}
    </section>
  )
}

export function KeysPage() {
  const { data, refetch } = useSettingsData()
  if (!data) return <LoadFailed words="settings_failed_safety" onRetry={() => void refetch()} />
  return (
    <div className="flex flex-col gap-6">
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
  const { data: s, refetch } = useSettingsData()
  const [tab, setTab] = useState<SafetyTab>('safety')
  if (!s) return <LoadFailed onRetry={() => void refetch()} />
  return <SafetyView s={s} tab={tab} onTab={setTab} />
}

export function FoldersPage() {
  return <SessionFoldersSection />
}

export function BackupPage() {
  const status = useBackupStatus()
  const hello = useQuery({ queryKey: ['hello'], queryFn: ({ signal }) => fetchHello(signal) }).data
  return <BackupView status={status} canStart={false} version={hello?.version ?? null} />
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
function useTrailing(hub: boolean, phone: boolean, backup: BackupStatus | null): Record<string, string> {
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
  return {
    notifications: hub ? '' : t(notif),
    time: !hqZone ? zoneCity(zone) : zonesDiffer(hqZone, zone) ? t('tz_row_differ', { city: zoneCity(zone), hqCity: zoneCity(hqZone) }) : t('tz_row_same', { city: zoneCity(zone) }),
    version: hello ? t('trail_version', { version: hello.version }) : '',
    data: phone ? t('data_used_row_window') : used === null ? '' : t('data_used_row', { size: formatSize(used) }),
    signout: t(hub ? 'trail_sign_out_window' : 'trail_sign_out_browser'),
    backup: backupRowWords(backup),
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

  const trailing = useTrailing(hub, phone, backup)
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
      <SettingsHome lite={lite} hub={hub} phone={phone} trailing={{ ...trailing, notifications: notificationsOff && !wide ? t('trail_notif_none') : trailing.notifications }} needsLook={needsLook} onSignOut={() => setSigningOut(true)} />
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
