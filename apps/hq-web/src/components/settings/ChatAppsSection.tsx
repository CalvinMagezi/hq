import { useEffect, useRef, useState, type ReactNode } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ExternalLink, Loader2 } from 'lucide-react'
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

const errorText = (e: unknown) => (e instanceof Error ? e.message : 'That did not work.')

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
      <>Open <ExtLink href="https://t.me/BotFather">@BotFather</ExtLink> in Telegram.</>,
      <>Send <code className="font-mono">/newbot</code>.</>,
      <>Pick a display name, then a username that ends in <code className="font-mono">bot</code>.</>,
      <>BotFather replies with a token. Copy it and paste it below.</>,
    ],
    pairLabel: 'Pair my Telegram',
    pairHelp: (p) =>
      p.deep_link ? (
        <>Tap the button to open your bot and press <strong>Start</strong>. Or send it <code className="font-mono select-all">{p.command}</code> yourself.</>
      ) : (
        <>Open your bot in Telegram and send it <code className="font-mono select-all">{p.command}</code>.</>
      ),
  },
  discord: {
    title: 'Discord',
    steps: [
      <>Open the <ExtLink href="https://discord.com/developers/applications">Discord developer portal</ExtLink> and choose New Application.</>,
      <>Open Bot, press Reset Token and copy the token.</>,
      <>On the same page, under Privileged Gateway Intents, turn on <strong>Message Content Intent</strong> and save.</>,
      <>Paste the token below.</>,
    ],
    pairLabel: 'Pair my Discord',
    pairHelp: (p) => (
      <>Open a direct message with your bot in Discord and send <code className="font-mono select-all">{p.command}</code>.</>
    ),
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
    setState(back ? { phase: 'done' } : { phase: 'failed', message: 'HQ did not come back within two minutes. Check the server.' })
  }
  return { state, run }
}

function RestartNotice({ restarter }: { restarter: Restarter }) {
  const { state } = restarter
  if (state.phase === 'idle') return null
  if (state.phase === 'restarting') {
    return (
      <p role="status" className="flex items-center gap-2 text-sm text-neutral-300">
        <Loader2 className="w-4 h-4 animate-spin" /> Restarting HQ. This page reconnects by itself.
      </p>
    )
  }
  if (state.phase === 'done') return <p role="status" className="text-sm text-neutral-300">HQ restarted with the new settings.</p>
  return <p role="alert" className="text-sm text-neutral-100">{state.message}</p>
}

function RelayLine({ app }: { app: ChatAppStatus }) {
  const relay = app.relay
  if (!app.enabled || !app.token_set) return <span>Off</span>
  if (app.needs_restart) return <span>Saved. Restart HQ to start it.</span>
  if (!relay || !relay.current) return <span>Not running yet</span>
  if (relay.error) return <span>Error: {relay.error}</span>
  return (
    <span>
      {relay.status === 'connected' ? 'Running' : relay.status}, seen {relTime(relay.last_seen)}
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
      if (!res.ok) throw new Error(res.error ?? 'That token did not work.')
      setChecked(res.username ?? null)
    })
  return (
    <div className="flex flex-col gap-3 py-3.5">
      <ol className="list-decimal pl-5 flex flex-col gap-1 text-sm text-neutral-300 leading-relaxed">
        {COPY[platform].steps.map((step, i) => <li key={i}>{step}</li>)}
      </ol>
      <label className="flex flex-col gap-1 text-xs text-neutral-400">
        Bot token
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
      {checked && <p className="text-sm text-neutral-100">Found your bot: @{checked}</p>}
      {error && <p role="alert" className="text-sm text-neutral-100 break-words">{error}</p>}
      <div className="flex flex-wrap gap-2">
        <button type="button" className="hq-btn-ghost disabled:opacity-40" disabled={busy || !token.trim()} onClick={check}>
          Check token
        </button>
        <button type="button" className="hq-btn-ghost disabled:opacity-40" disabled={busy || checked === null} onClick={() => run(() => onSave(token.trim()))}>
          {busy && <Loader2 className="w-3.5 h-3.5 animate-spin" />} Save and restart
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
        The code expired before it was used.
        <button type="button" className="hq-btn-ghost self-start" onClick={onCancel}>Close</button>
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-3 py-3.5 text-sm text-neutral-300">
      {pair.deep_link && (
        <a href={pair.deep_link} target="_blank" rel="noreferrer" className="hq-btn-ghost self-start px-4 py-2 text-sm">
          <ExternalLink className="w-4 h-4" /> Open in Telegram
        </a>
      )}
      <p className="leading-relaxed">{COPY[platform].pairHelp(pair)}</p>
      <p className="font-mono text-2xl tracking-widest text-neutral-50 select-all">{pair.code}</p>
      <p className="flex items-center gap-2">
        <Loader2 className="w-3.5 h-3.5 animate-spin" /> Waiting for the message. The code works for {formatCountdown(left)}.
      </p>
      <button type="button" className="hq-btn-ghost self-start" onClick={onCancel}>Cancel</button>
    </div>
  )
}

function DiscordInvite({ app }: { app: ChatAppStatus }) {
  const appId = app.bot?.application_id
  if (!appId) return null
  return (
    <div className="flex flex-col gap-2 py-3.5 border-t border-white/10 text-sm text-neutral-300">
      <span className="text-neutral-300">Add the bot to a server you own</span>
      <a href={discordInviteUrl(appId)} target="_blank" rel="noreferrer" className="hq-btn-ghost self-start">
        <ExternalLink className="w-3.5 h-3.5" /> Open the invite link
      </a>
      {app.bot?.message_content === false && (
        <p role="alert" className="text-neutral-100">
          Message Content Intent looks off. Turn it on under Bot, Privileged Gateway Intents, or the bot cannot read messages.
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
  if (app.owner.from_config) return <Line label="Owner">set in the config file</Line>
  if (!app.owner.paired) {
    return (
      <Line label="Owner">
        <button type="button" className="hq-btn-ghost" disabled={busy} onClick={() => onPair(false)}>{COPY[platform].pairLabel}</button>
      </Line>
    )
  }
  return (
    <Line label="Owner">
      <span className="inline-flex flex-wrap items-baseline justify-end gap-x-3 gap-y-1">
        <span>Paired{app.owner.id ? ` (id ${app.owner.id})` : ''}</span>
        <button type="button" className="hq-btn-ghost" disabled={busy} onClick={() => onPair(true)}>Pair a different account</button>
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
    <Line label="Restart">
      {data.can_restart ? (
        <button type="button" className="hq-btn-ghost" disabled={restarter.state.phase === 'restarting'} onClick={() => restarter.run()}>Restart HQ now</button>
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
  const disconnect = () => {
    if (!window.confirm(`Disconnect the ${COPY[platform].title} bot from HQ?`)) return
    run(async () => applyChange((await disconnectChatApp(platform)).can_restart))
  }
  const startPairing = (replace: boolean) => {
    if (replace && !window.confirm('The current owner loses access until the new account pairs. Continue?')) return
    run(async () => setPair(await pairChatApp(platform, replace)))
  }
  const recheck = () =>
    run(async () => {
      const res = await verifyChatToken(platform)
      if (!res.ok) throw new Error(res.error ?? 'The saved token did not work.')
    })

  const envLocked = app.token_from_env || app.enabled_from_env
  const connected = app.token_set && app.enabled
  const editable = data.writable && !envLocked
  const save = (token: string) => run(async () => applyChange((await saveChatToken(platform, token)).can_restart))

  return (
    <div className="hq-card rounded-2xl px-5">
      <Line label={COPY[platform].title}><RelayLine app={app} /></Line>
      {envLocked && <Line label="Token">Set by the server's environment; change it there.</Line>}
      {!connected && editable && <ConnectForm platform={platform} onSave={save} />}
      {connected && (
        <Line label="Bot">
          {app.bot ? `@${app.bot.username}` : (
            <button type="button" className="hq-btn-ghost" disabled={busy || !data.writable} onClick={recheck}>Check the bot</button>
          )}
        </Line>
      )}
      {connected && platform === 'discord' && <DiscordInvite app={app} />}
      {connected && data.writable && !pair && <OwnerLine platform={platform} app={app} busy={busy} onPair={startPairing} />}
      {pair && <PairPanel platform={platform} pair={pair} onCancel={() => setPair(null)} />}
      {app.needs_restart && data.writable && <RestartLine data={data} restarter={restarter} />}
      {connected && editable && (
        <Line label="Disconnect">
          <button type="button" className="hq-btn-ghost" disabled={busy} onClick={disconnect}>Disconnect</button>
        </Line>
      )}
      {error && <p role="alert" className="py-3 text-sm text-neutral-100 break-words border-t border-white/10">{error}</p>}
    </div>
  )
}


/** Connect a Telegram or Discord bot and pair its owner, with no shell on the server. */
export function ChatAppsSection() {
  const { data } = useQuery({ queryKey: CHAT_APPS_QUERY_KEY, queryFn: fetchChatApps })
  const restarter = useRestarter(data?.started_at)
  if (!data) return null
  return (
    <section id="chat-apps" aria-labelledby="chat-apps-title" className="scroll-mt-4 flex flex-col gap-3">
      <h2 id="chat-apps-title" className="px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-400">
        Chat apps
      </h2>
      {!data.writable && (
        <p className="px-1 text-sm text-neutral-400">
          Changing chat apps from here needs a web token (web_auth_token). Until then, edit the relay section of the config on the server.
        </p>
      )}
      <RestartNotice restarter={restarter} />
      <ChatAppCard platform="telegram" data={data} restarter={restarter} />
      <ChatAppCard platform="discord" data={data} restarter={restarter} />
    </section>
  )
}
