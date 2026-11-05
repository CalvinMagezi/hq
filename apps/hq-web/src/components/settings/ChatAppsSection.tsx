import { useEffect, useRef, useState, type ReactNode } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ExternalLink, Loader2 } from 'lucide-react'
import { useSharedConnection } from '~/lib/connection'
import { Dialog } from '~/components/ui/Dialog'
import { t } from '~/strings'
import { relTime } from '~/lib/time'
import {
  CHAT_APPS_QUERY_KEY,
  disconnectChatApp,
  discordInviteUrl,
  fetchChatApps,
  formatCountdown,
  pairChatApp,
  restartHq,
  saveChatToken,
  secondsLeft,
  verifyChatToken,
  waitForRestart,
  type ChatAppStatus,
  type ChatAppsStatus,
  type ChatPlatform,
  type PairResult,
} from '~/lib/chatAppsApi'

const PAIRING_POLL_MS = 3000
const TICK_MS = 1000

const errorText = (e: unknown) => (e instanceof Error ? e.message : t('settings_app_error'))

/** Runs one action at a time and keeps its error for display. `after` runs whether it worked or not. */
function useBusyAction(after?: () => Promise<unknown>) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = async (action: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try {
      await action()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
      await after?.()
    }
  }
  return { busy, error, run }
}

function ExtLink({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a href={href} target="_blank" rel="noreferrer" className="underline text-neutral-100 hover:text-white">
      {children}
    </a>
  )
}

interface PlatformCopy {
  title: string
  steps: ReactNode[]
  pairLabel: string
  pairHelp: (p: PairResult) => ReactNode
}

const COPY: Record<ChatPlatform, PlatformCopy> = {
  telegram: {
    title: 'Telegram',
    steps: [
      <ExtLink href="https://t.me/BotFather">{t('settings_app_step_botfather')}</ExtLink>,
      <>{t('settings_app_step_newbot')}</>,
      <>{t('settings_app_step_name')}</>,
      <>{t('settings_app_step_token')}</>,
    ],
    pairLabel: t('settings_app_pair', { app: 'Telegram' }),
    pairHelp: (p) =>
      p.deep_link ? (
        <>{t('settings_app_pair_telegram', { command: p.command })}</>
      ) : (
        <>{t('settings_app_pair_telegram', { command: p.command })}</>
      ),
  },
  discord: {
    title: 'Discord',
    steps: [
      <ExtLink href="https://discord.com/developers/applications">{t('settings_app_step_discord_portal')}</ExtLink>,
      <>{t('settings_app_step_discord_token')}</>,
      <>{t('settings_app_step_discord_intent')}</>,
      <>{t('settings_app_step_discord_paste')}</>,
    ],
    pairLabel: t('settings_app_pair', { app: 'Discord' }),
    pairHelp: (p) => <>{t('settings_app_pair_discord', { command: p.command })}</>,
  },
}

type RestartPhase = { phase: 'idle' } | { phase: 'restarting' } | { phase: 'done' } | { phase: 'failed'; message: string }

interface Restarter {
  state: RestartPhase
  run: () => Promise<void>
}

/** Restarts HQ and waits for the new process, so the cards never show the old one as back. */
function useRestarter(startedAt: number | undefined): Restarter {
  const client = useQueryClient()
  const [state, setState] = useState<RestartPhase>({ phase: 'idle' })
  const run = async () => {
    if (startedAt === undefined) return
    setState({ phase: 'restarting' })
    try {
      await restartHq()
    } catch (e) {
      setState({ phase: 'failed', message: errorText(e) })
      return
    }
    const back = await waitForRestart(startedAt)
    await client.invalidateQueries({ queryKey: CHAT_APPS_QUERY_KEY })
    setState(back ? { phase: 'done' } : { phase: 'failed', message: t('settings_app_restart_failed') })
  }
  return { state, run }
}

function RestartNotice({ restarter }: { restarter: Restarter }) {
  const { state } = restarter
  if (state.phase === 'idle') return null
  if (state.phase === 'restarting') {
    return (
      <p role="status" className="flex items-center gap-2 text-sm text-neutral-300">
        <Loader2 className="w-4 h-4 animate-spin" /> {t('settings_app_restarting')}
      </p>
    )
  }
  if (state.phase === 'done') return <p role="status" className="text-sm text-neutral-300">{t('settings_app_restarted')}</p>
  return <p role="alert" className="text-sm text-neutral-100">{state.message}</p>
}

function RelayLine({ app }: { app: ChatAppStatus }) {
  const relay = app.relay
  if (!app.enabled || !app.token_set) return <span>{t('settings_app_off')}</span>
  if (app.needs_restart) return <span>{t('settings_app_needs_restart')}</span>
  if (!relay || !relay.current) return <span>{t('settings_app_not_running')}</span>
  if (relay.error) return <span>{relay.error}</span>
  return (
    <span>
      {relay.status === 'connected' ? t('settings_app_running') : relay.status}, {t('settings_app_seen', { age: relTime(relay.last_seen) })}
    </span>
  )
}

function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    const id = setInterval(() => setNow(Date.now()), TICK_MS)
    return () => clearInterval(id)
  }, [active])
  return now
}

function ConnectForm({ platform, onSave }: { platform: ChatPlatform; onSave: (token: string) => Promise<void> }) {
  const [token, setToken] = useState('')
  const [checked, setChecked] = useState<string | null>(null)
  const { busy, error, run } = useBusyAction()
  const check = () =>
    run(async () => {
      const res = await verifyChatToken(platform, token.trim())
      if (!res.ok) throw new Error(res.error ?? t('settings_app_token_bad'))
      setChecked(res.username ?? null)
    })
  return (
    <div className="flex flex-col gap-3 py-3.5">
      <ol className="list-decimal pl-5 flex flex-col gap-1 text-sm text-neutral-300 leading-relaxed">
        {COPY[platform].steps.map((step, i) => <li key={i}>{step}</li>)}
      </ol>
      <label className="flex flex-col gap-1 text-xs text-neutral-400">
        {t('settings_app_bot')}
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={token}
          onChange={(e) => {
            setToken(e.target.value)
            setChecked(null)
          }}
          className="rounded-lg bg-neutral-900 border border-white/10 px-3 py-2 text-neutral-100 font-mono"
        />
      </label>
      {checked && <p className="text-sm text-neutral-100">{t('settings_app_found_bot', { name: checked })}</p>}
      {error && <p role="alert" className="text-sm text-neutral-100 break-words">{error}</p>}
      <div className="flex flex-wrap gap-2">
        <button type="button" className="hq-btn-ghost disabled:opacity-40" disabled={busy || !token.trim()} onClick={check}>
          {t('settings_app_check_bot')}
        </button>
        <button type="button" className="hq-btn-ghost disabled:opacity-40" disabled={busy || checked === null} onClick={() => run(() => onSave(token.trim()))}>
          {busy && <Loader2 className="w-3.5 h-3.5 animate-spin" />} {t('settings_app_save_restart')}
        </button>
      </div>
    </div>
  )
}

function PairPanel({ platform, pair, onCancel }: { platform: ChatPlatform; pair: PairResult; onCancel: () => void }) {
  const left = secondsLeft(pair.expires_at, useNow(true))
  if (left === 0) {
    return (
      <div className="flex flex-col gap-2 py-3.5 text-sm text-neutral-300">
        {t('settings_app_pair_expired')}
        <button type="button" className="hq-btn-ghost self-start" onClick={onCancel}>Close</button>
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-3 py-3.5 text-sm text-neutral-300">
      {pair.deep_link && (
        <a href={pair.deep_link} target="_blank" rel="noreferrer" className="hq-btn-ghost self-start px-4 py-2 text-sm">
          <ExternalLink className="w-4 h-4" /> {t('settings_app_open_telegram')}
        </a>
      )}
      <p className="leading-relaxed">{COPY[platform].pairHelp(pair)}</p>
      <p className="font-mono text-2xl tracking-widest text-neutral-50 select-all">{pair.code}</p>
      <p className="flex items-center gap-2">
        <Loader2 className="w-3.5 h-3.5 animate-spin" /> {t('settings_app_pair_waiting', { time: formatCountdown(left) })}
      </p>
      <button type="button" className="hq-btn-ghost self-start" onClick={onCancel}>{t('spend_cancel')}</button>
    </div>
  )
}

function DiscordInvite({ app }: { app: ChatAppStatus }) {
  const appId = app.bot?.application_id
  if (!appId) return null
  return (
    <div className="flex flex-col gap-2 py-3.5 border-t border-white/10 text-sm text-neutral-300">
      <span className="text-neutral-300">{t('settings_app_invite')}</span>
      <a href={discordInviteUrl(appId)} target="_blank" rel="noreferrer" className="hq-btn-ghost self-start">
        <ExternalLink className="w-3.5 h-3.5" /> {t('settings_app_invite_open')}
      </a>
      {app.bot?.message_content === false && (
        <p role="alert" className="text-neutral-100">
          {t('settings_app_intent_off')}
        </p>
      )}
    </div>
  )
}

function Line({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-4 py-3.5 text-sm border-t border-white/10 first:border-t-0">
      <span className="text-neutral-300 shrink-0">{label}</span>
      <span className="min-w-0 break-words text-right text-neutral-50">{children}</span>
    </div>
  )
}

interface CardProps {
  platform: ChatPlatform
  data: ChatAppsStatus
  restarter: Restarter
}

function OwnerLine({ platform, app, onPair, busy }: { platform: ChatPlatform; app: ChatAppStatus; onPair: (replace: boolean) => void; busy: boolean }) {
  if (app.owner.from_config) return <Line label={t('settings_app_account')}>{t('settings_app_linked')}</Line>
  if (!app.owner.paired) {
    return (
      <Line label={t('settings_app_account')}>
        <button type="button" className="hq-btn-ghost" disabled={busy} onClick={() => onPair(false)}>{COPY[platform].pairLabel}</button>
      </Line>
    )
  }
  return (
    <Line label={t('settings_app_account')}>
      <span className="inline-flex flex-wrap items-baseline justify-end gap-x-3 gap-y-1">
        <span>{t('settings_app_linked')}</span>
        <button type="button" className="hq-btn-ghost" disabled={busy} onClick={() => onPair(true)}>{t('settings_app_link_other')}</button>
      </span>
    </Line>
  )
}

/** The open pairing code, polled for while it waits. */
function usePairing(paired: boolean) {
  const [pair, setPair] = useState<PairResult | null>(null)
  useQuery({ queryKey: CHAT_APPS_QUERY_KEY, queryFn: fetchChatApps, refetchInterval: pair ? PAIRING_POLL_MS : false })
  // Close the panel only when an owner appears, not because one was paired when it opened (a re-pair).
  const wasPaired = useRef(paired)
  useEffect(() => {
    if (pair && paired && !wasPaired.current) setPair(null)
    wasPaired.current = paired
  }, [pair, paired])
  return [pair, setPair] as const
}

function RestartLine({ data, restarter }: { data: ChatAppsStatus; restarter: Restarter }) {
  return (
    <Line label={t('settings_app_needs_restart')}>
      {data.can_restart ? (
        <button type="button" className="hq-btn-ghost" disabled={restarter.state.phase === 'restarting'} onClick={() => restarter.run()}>{t('settings_app_restart_now')}</button>
      ) : (
        <span className="text-neutral-300">{data.restart_hint}</span>
      )}
    </Line>
  )
}

function ChatAppCard({ platform, data, restarter }: CardProps) {
  const client = useQueryClient()
  const app = data[platform]
  const [pair, setPair] = usePairing(app.owner.paired)
  const { busy, error, run } = useBusyAction(() => client.invalidateQueries({ queryKey: CHAT_APPS_QUERY_KEY }))
  const applyChange = async (canRestart: boolean) => {
    if (canRestart) await restarter.run()
  }
  const [asking, setAsking] = useState<'disconnect' | 'unlink' | null>(null)
  const appName = COPY[platform].title
  const disconnect = () => {
    setAsking(null)
    run(async () => applyChange((await disconnectChatApp(platform)).can_restart))
  }
  const startPairing = (replace: boolean) => {
    setAsking(null)
    run(async () => setPair(await pairChatApp(platform, replace)))
  }
  const recheck = () =>
    run(async () => {
      const res = await verifyChatToken(platform)
      if (!res.ok) throw new Error(res.error ?? t('settings_app_token_bad'))
    })

  const envLocked = app.token_from_env || app.enabled_from_env
  const connected = app.token_set && app.enabled
  const reachable = useSharedConnection() === 'connected'
  const editable = data.writable && !envLocked
  const save = (token: string) => run(async () => applyChange((await saveChatToken(platform, token)).can_restart))

  return (
    <div className="hq-card rounded-2xl px-5">
      <Line label={COPY[platform].title}><RelayLine app={app} /></Line>
      {envLocked && <Line label={t('settings_app_bot')}>{t('settings_app_env_locked')}</Line>}
      {!connected && editable && <ConnectForm platform={platform} onSave={save} />}
      {connected && (
        <Line label={t('settings_app_bot')}>
          {app.bot ? `@${app.bot.username}` : (
            <button type="button" className="hq-btn-ghost" disabled={busy || !data.writable} onClick={recheck}>{t('settings_app_check_bot')}</button>
          )}
        </Line>
      )}
      {connected && platform === 'discord' && <DiscordInvite app={app} />}
      {connected && data.writable && !pair && <OwnerLine platform={platform} app={app} busy={busy} onPair={(replace) => (replace ? setAsking('unlink') : startPairing(false))} />}
      {pair && <PairPanel platform={platform} pair={pair} onCancel={() => setPair(null)} />}
      {app.needs_restart && data.writable && <RestartLine data={data} restarter={restarter} />}
      {connected && editable && (
        <Line label={t('settings_app_disconnect')}>
          <button type="button" className="hq-btn-ghost" disabled={busy || !reachable} onClick={() => setAsking('disconnect')}>{t('settings_app_disconnect')}</button>
          {!reachable && <span className="block a-muted a-body-sm">{t('settings_needs_hq_unreachable')}</span>}
        </Line>
      )}
      {error && <p role="alert" className="py-3 text-sm text-neutral-100 break-words border-t border-white/10">{error}</p>}
      {asking === 'disconnect' && (
        <Dialog
          kind="red"
          title={t('disconnect_app_title', { app: appName })}
          body={t('disconnect_app_body', { app: appName })}
          keepLabel={t('disconnect_app_keep', { app: appName })}
          actLabel={t('disconnect_app_act', { app: appName })}
          busyLabel={t('confirm_busy_disconnecting')}
          busy={busy}
          onKeep={() => setAsking(null)}
          onAct={disconnect}
        />
      )}
      {asking === 'unlink' && (
        <Dialog
          kind="red"
          title={t('unlink_account_title', { app: appName })}
          body={t('unlink_account_body', { app: appName })}
          keepLabel={t('unlink_account_keep')}
          actLabel={t('unlink_account_act')}
          busyLabel={t('confirm_busy_unlinking')}
          busy={busy}
          onKeep={() => setAsking(null)}
          onAct={() => startPairing(true)}
        />
      )}
    </div>
  )
}


/** Connect a Telegram or Discord bot and pair its owner, with no shell on the server. */
export function ChatAppsSection() {
  const { data, isError, refetch } = useQuery({ queryKey: CHAT_APPS_QUERY_KEY, queryFn: fetchChatApps })
  const restarter = useRestarter(data?.started_at)
  if (!data) {
    return isError ? (
      <div className="a-card" role="status">
        <b>{t('settings_failed_messaging')}</b>
        <span className="a-muted">{t('settings_failed_body')}</span>
        <button type="button" className="a-btn secondary" onClick={() => void refetch()}>
          {t('settings_try_again')}
        </button>
      </div>
    ) : (
      <p className="a-muted" role="status">{t('settings_loading_messaging')}</p>
    )
  }
  return (
    <section id="chat-apps" aria-labelledby="chat-apps-title" className="scroll-mt-4 flex flex-col gap-3">
      <h2 id="chat-apps-title" className="px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400">
        {t('row_messaging')}
      </h2>
      {!data.writable && (
        <p className="px-1 text-sm text-neutral-400">
          {t('settings_app_needs_token')}
        </p>
      )}
      <RestartNotice restarter={restarter} />
      <ChatAppCard platform="telegram" data={data} restarter={restarter} />
      <ChatAppCard platform="discord" data={data} restarter={restarter} />
    </section>
  )
}
