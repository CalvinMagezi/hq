import { Dialog } from '~/components/ui/Dialog'
import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { formatSize } from '~/lib/settingsGroups'
import { readDataUsed } from '~/lib/dataUsed'
import { lastHourBytes, overIdleTarget, readFirstLoad } from '~/lib/dataUsage'
import { fetchHello } from '~/lib/hello'
import { useInHub } from '~/lib/hubWindow'
import { APP_VERSION, reloadForNewer, useNewerHq } from '~/lib/version'
import { HqHttpError, refuseSignIn } from '~/lib/hqAuth'
import { signOutCurrentDevice } from '~/lib/deviceSignIn'
import { useInstallOffer } from '~/components/InstallPrompt'
import { t } from '~/strings'
import { supportDetails } from '~/lib/supportDetails'

const HTTP_BAD_REQUEST = 400

/** Install and version: the browser's install offer and the HQ version, with Reload when a newer one waits. */
export function VersionPage() {
  const { canInstall, install } = useInstallOffer()
  const newer = useNewerHq()
  const hello = useQuery({ queryKey: ['hello'], queryFn: ({ signal }) => fetchHello(signal) }).data
  return (
    <div className="flex flex-col gap-4">
      {newer && (
        <div className="a-card warn" role="status">
          <b>{t('card_newer_title')}</b>
          <span className="a-muted">{t('card_newer_body')}</span>
          <button type="button" className="a-btn" onClick={() => void reloadForNewer()}>
            {t('card_newer_act')}
          </button>
        </div>
      )}
      <div className="a-card">
        <b>{t('ver_install_title')}</b>
        <span className="a-muted">{t('ver_install_body')}</span>
        <span className="a-muted">{t('ver_install_ios')}</span>
        {canInstall && (
          <button type="button" className="a-btn secondary" onClick={() => void install()}>
            {t('ver_install_act')}
          </button>
        )}
      </div>
      <div className="a-card">
        <b>{t('ver_version_title')}</b>
        <span className="a-muted">{t('ver_browser_line', { version: APP_VERSION })}</span>
        {hello && <span className="a-muted">{t('ver_hq_line', { version: hello.version })}</span>}
      </div>
      <div className="a-card">
        <b>{t('ver_updates_title')}</b>
        <span className="a-muted">{t('ver_updates_body')}</span>
        <span className="a-muted">{t('ver_updates_value')}</span>
      </div>
      <SupportCard lines={[t('ver_browser_line', { version: APP_VERSION }), hello ? t('ver_hq_line', { version: hello.version }) : '']} />
    </div>
  )
}

function SupportCard({ lines }: { lines: string[] }) {
  const [text, setText] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  return (
    <div className="a-card">
      <b>{t('copy_details_title')}</b>
      <span className="a-muted">{t('ver_support_body')}</span>
      {text === null ? (
        <button type="button" className="a-btn secondary" onClick={() => setText(supportDetails(lines))}>
          {t('copy_details_title')}
        </button>
      ) : (
        <>
          <span className="a-muted">{t('copy_details_body')}</span>
          <pre className="a-body-sm whitespace-pre-wrap break-words">{text}</pre>
          <div className="flex gap-2">
            <button type="button" className="a-btn" onClick={() => void navigator.clipboard?.writeText(text).then(() => setCopied(true))}>
              {t('copy_details_copy')}
            </button>
            <button type="button" className="a-btn secondary" onClick={() => setText(null)}>
              {t('copy_details_not_now')}
            </button>
          </div>
          {copied && <span role="status" className="a-muted">{t('copy_details_copied')}</span>}
        </>
      )}
    </div>
  )
}

/** Data used by HQ in this browser this month: the month, an idle hour and the Chat first load. */
export function DataUsedPage() {
  const hub = useInHub()
  const [numbers, setNumbers] = useState<{ month: number; idle: number | null; first: number | null } | null>(null)
  useEffect(() => setNumbers({ month: readDataUsed(), idle: lastHourBytes(), first: readFirstLoad() }), [])
  const idleOver = numbers?.idle != null && overIdleTarget(numbers.idle)
  return (
    <div className="flex flex-col gap-4">
      <div className="a-card">
        <b>{hub ? t('data_phone') : t('data_month_title')}</b>
        <span className="a-muted">{numbers ? formatSize(numbers.month) : ''}</span>
      </div>
      <div className={idleOver ? 'a-card warn' : 'a-card'}>
        <b>{t('data_idle_title')}</b>
        <span className="a-muted">{t('data_idle_body')}</span>
        <span className="a-muted">{numbers?.idle != null ? formatSize(numbers.idle) : ''}</span>
        {idleOver && <span className="a-muted">{t('data_idle_over')}</span>}
      </div>
      <div className="a-card">
        <b>{t('data_chat_first')}</b>
        <span className="a-muted">{numbers?.first != null ? formatSize(numbers.first) : ''}</span>
      </div>
      <p className="a-muted a-body-sm">{t('data_note')}</p>
    </div>
  )
}

/** The stacked dialog behind Sign out this browser: signs it out on HQ, then wipes what it saved. */
export function SignOutDialog({ onClose }: { onClose: () => void }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const signOut = async () => {
    setBusy(true)
    setError(null)
    try {
      await signOutCurrentDevice()
    } catch (e) {
      // The operator link has no device of its own to remove; only Sign out everywhere ends it.
      const operatorLink = e instanceof HqHttpError && e.status === HTTP_BAD_REQUEST
      setError(operatorLink ? t('sign_out_browser_operator') : t('sign_out_browser_failed'))
      setBusy(false)
      return
    }
    onClose()
    refuseSignIn()
  }
  return (
    <Dialog
      kind="red"
      title={t('signout_browser_title')}
      body={t('signout_browser_body')}
      keepLabel={t('sign_out_device_keep')}
      actLabel={t('row_sign_out_browser')}
      busyLabel={t('confirm_busy_signing_out')}
      busy={busy}
      error={error}
      onKeep={onClose}
      onAct={() => void signOut()}
    />
  )
}
