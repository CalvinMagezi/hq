import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ErrorText } from '~/components/sessions/ErrorText'
import {
  HELPER_SETUP_KEY,
  anyRunning,
  fetchHelperSetup,
  helperState,
  runHelperSetup,
  type SetupAction,
  type SetupHelper,
  type SetupHost,
} from '~/lib/helperSetup'
import { relTime } from '~/lib/time'
import { t } from '~/strings'

const IDLE_POLL_MS = 30_000
const RUNNING_POLL_MS = 3_000

const STATE_WORDS = {
  ready: 'helper_setup_state_ready',
  signed_out: 'helper_setup_state_signed_out',
  unknown_login: 'helper_setup_state_unknown_login',
  missing: 'helper_setup_state_missing',
  unchecked: 'helper_setup_state_unchecked',
} as const

interface RowProps {
  helper: SetupHelper
  busy: boolean
  onAct: (action: SetupAction) => void
}

function HelperRow({ helper: h, busy, onAct }: RowProps) {
  const state = helperState(h)
  const running = h.run?.state === 'running'
  const failed = h.run?.state === 'failed' && !running
  return (
    <li className="a-card">
      <div className="a-card-head">
        <b className="a-body-b">{h.title}</b>
        <span className="a-count">{t(STATE_WORDS[state])}</span>
      </div>
      <p className="a-body-sm">{h.about}</p>
      <p className="a-body-sm a-muted">{h.free}</p>
      {running && (
        <div role="status">
          <p className="a-body-sm">{t('helper_setup_running')}</p>
          {h.run?.urls.map((url) => (
            <a key={url} href={url} target="_blank" rel="noopener noreferrer" className="a-btn secondary compact">
              {t('helper_setup_link')}
            </a>
          ))}
        </div>
      )}
      {(failed || h.error) && !running && (
        <p className="a-body-sm a-warn-t">{h.error ?? t('helper_setup_failed')}</p>
      )}
      {!running && state !== 'ready' && (
        <div className="a-btns">
          {(state === 'missing' || state === 'unchecked') && (
            <button type="button" className="a-btn compact" disabled={busy} onClick={() => onAct('setup')}>
              {t('helper_setup_one')}
            </button>
          )}
          {(state === 'signed_out' || state === 'unknown_login') && (
            <button type="button" className="a-btn compact" disabled={busy} onClick={() => onAct('login')}>
              {t('helper_setup_signin')}
            </button>
          )}
        </div>
      )}
    </li>
  )
}

function HostBlock({ host, onRun, busy }: { host: SetupHost; onRun: (a: SetupAction, ids: string[]) => void; busy: boolean }) {
  const missing = host.helpers.filter((h) => helperState(h) !== 'ready').map((h) => h.id)
  const checkedAt = Math.max(0, ...host.helpers.map((h) => h.checked_at ?? 0))
  return (
    <section aria-label={t('helper_setup_computer', { host: host.host })}>
      <p className="a-grp">{t('helper_setup_computer', { host: host.host })}</p>
      <ul className="hq-chat-side" style={{ padding: 0 }}>
        {host.helpers.map((h) => (
          <HelperRow key={h.id} helper={h} busy={busy} onAct={(a) => onRun(a, [h.id])} />
        ))}
      </ul>
      <div className="a-btns">
        {missing.length > 1 && (
          <button type="button" className="a-btn" disabled={busy} onClick={() => onRun('setup', missing)}>
            {t('helper_setup_all')}
          </button>
        )}
        <button type="button" className="a-btn quiet compact" disabled={busy} onClick={() => onRun('check', [])}>
          {t('helper_setup_check')}
        </button>
        {checkedAt > 0 && <span className="a-body-sm a-muted">{t('helper_setup_checked', { when: relTime(new Date(checkedAt * 1000)) })}</span>}
      </div>
    </section>
  )
}

/** First-run setup of the free coding helpers on each computer HQ has a host on. */
export function HelperSetup() {
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const list = useQuery({
    queryKey: HELPER_SETUP_KEY,
    queryFn: () => fetchHelperSetup(),
    refetchInterval: (q) => (anyRunning(q.state.data ?? []) ? RUNNING_POLL_MS : IDLE_POLL_MS),
  })
  const hosts = list.data ?? []

  const run = async (host: string, action: SetupAction, ids: string[]) => {
    setBusy(true)
    setError(null)
    try {
      await runHelperSetup(host, action, ids)
    } catch (err) {
      setError(err instanceof Error ? err.message : t('helper_setup_failed'))
    } finally {
      setBusy(false)
      await client.invalidateQueries({ queryKey: HELPER_SETUP_KEY })
    }
  }

  if (list.isLoading) return null
  // Everything is ready: nothing to set up, so the page stays about the work.
  if (hosts.length > 0 && hosts.every((h) => h.helpers.every((x) => helperState(x) === 'ready'))) return null
  return (
    <div className="a-card mx-4 mb-3" aria-label={t('helper_setup_title')}>
      <b className="a-body-b">{t('helper_setup_title')}</b>
      <p className="a-body-sm">{t('helper_setup_intro')}</p>
      {hosts.length === 0 && <p className="a-body-sm a-muted">{t('helper_setup_none_connected')}</p>}
      {hosts.map((host) => (
        <HostBlock key={host.host} host={host} busy={busy} onRun={(a, ids) => void run(host.host, a, ids)} />
      ))}
      {error && <ErrorText>{error}</ErrorText>}
    </div>
  )
}
