import {
  HeadContent,
  Link,
  Outlet,
  useNavigate,
  Scripts,
  createRootRoute,
} from '@tanstack/react-router'
import { PersistQueryClientProvider } from '@tanstack/react-query-persist-client'
import { CACHE_BUSTER, CACHE_MAX_AGE_MS, lastSyncedAt, queryClient, queryPersister } from '~/lib/offlineCache'
import type { ReactNode } from 'react'
import { useEffect, useState } from 'react'
import { MessageSquare, Search, Settings } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { WebSocketProvider } from '~/context/WebSocketContext'
import { SearchOverlay } from '~/components/SearchOverlay'
import { QuickNoteOverlay } from '~/components/QuickNoteOverlay'
import { GlobalChatOverlay } from '~/components/chat/GlobalChatOverlay'
import { InstallPrompt } from '~/components/InstallPrompt'
import { AppRail } from '~/components/AppRail'
import { BottomNav } from '~/components/BottomNav'
import { useNeedsYouCount } from '~/components/sessions/useNeedsYouCount'
import appCss from '../../app.css?url'
import { relTime } from '~/lib/time'
import { openrouterChipLabel, openrouterChipTitle, OPENROUTER_USAGE_POLL_MS, type OpenRouterUsageResponse } from '~/lib/openrouterUsageApi'
import { budgetChipLabel, budgetChipTitle, USAGE_POLL_MS, type BudgetsResponse } from '~/lib/usageApi'
import { chipLabel, chipTitle, COPILOT_USAGE_POLL_MS, type CopilotUsage } from '~/lib/copilotUsageApi'
import { hqJson } from '~/lib/hqAuth'
import { fetchSetupStatus, shouldRedirectToSetup } from '~/lib/setupApi'

const SYSTEM_NOTICE_DISMISS_MS = 6_000

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      { name: 'viewport', content: 'width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content' },
      { title: 'Agent HQ Vault' },
      { name: 'theme-color', content: '#000000' },
      { name: 'apple-mobile-web-app-capable', content: 'yes' },
      { name: 'apple-mobile-web-app-status-bar-style', content: 'black-translucent' },
    ],
    links: [
      { rel: 'stylesheet', href: appCss },
      { rel: 'manifest', href: '/manifest.json' },
      { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/icons/hq-favicon-32.png' },
      { rel: 'icon', type: 'image/png', sizes: '192x192', href: '/icons/hq-icon-192.png' },
      { rel: 'apple-touch-icon', sizes: '180x180', href: '/icons/hq-apple-touch-180.png' },
    ],
  }),
  component: RootComponent,
})

function RootComponent() {
  return (
    <RootDocument>
      <WebSocketProvider>
        <SetupRedirect />
        <VaultShell />
        <SearchOverlay />
        <QuickNoteOverlay />
        <GlobalChatOverlay />
      </WebSocketProvider>
    </RootDocument>
  )
}

function SetupRedirect() {
  // An effect, not useQuery, for the same prerender reason as CopilotChip. Checked once per load.
  const navigate = useNavigate()
  useEffect(() => {
    if (window.location.pathname === '/setup') return
    fetchSetupStatus()
      .then((status) => {
        if (shouldRedirectToSetup(status)) void navigate({ to: '/setup' })
      })
      .catch(() => undefined)
  }, [navigate])
  return null
}

function CopilotChip() {
  // A plain effect, not useQuery: a query hook in the root shell kept the build's prerender from
  // exiting, and effects never run there.
  const [data, setData] = useState<CopilotUsage | undefined>(undefined)
  const [openrouter, setOpenrouter] = useState<OpenRouterUsageResponse | undefined>(undefined)
  const [budgets, setBudgets] = useState<BudgetsResponse | undefined>(undefined)
  useEffect(() => {
    let alive = true
    const load = () => {
      hqJson<CopilotUsage>('/api/copilot-usage')
        .then((d) => alive && setData(d))
        .catch(() => undefined)
      hqJson<OpenRouterUsageResponse>('/api/openrouter-usage')
        .then((d) => alive && setOpenrouter(d))
        .catch(() => undefined)
      hqJson<BudgetsResponse>('/api/budgets')
        .then((d) => alive && setBudgets(d))
        .catch(() => undefined)
    }
    load()
    const timer = window.setInterval(load, Math.min(COPILOT_USAGE_POLL_MS, OPENROUTER_USAGE_POLL_MS, USAGE_POLL_MS))
    return () => {
      alive = false
      window.clearInterval(timer)
    }
  }, [])
  const copilotLabel = chipLabel(data)
  // A budget that needs attention outranks a balance: it is the thing about to block a call.
  const budgetLabel = budgetChipLabel(budgets)
  const label = budgetLabel ?? copilotLabel ?? openrouterChipLabel(openrouter)
  const title = budgetLabel ? budgetChipTitle(budgets) : copilotLabel ? chipTitle(data) : openrouterChipTitle(openrouter)
  if (!label) return null
  return (
    <Link
      to={budgetLabel ? '/usage' : '/settings'}
      title={title}
      className="hidden min-[400px]:inline px-2 py-0.5 rounded-md border border-white/10 text-[10px] font-mono text-neutral-400 hover:text-neutral-100 hover:bg-white/5 whitespace-nowrap"
    >
      {label}
    </Link>
  )
}

function VaultShell() {
  const wsConnected = useHQStore((s) => s.wsConnected)
  const setGlobalChatOpen = useHQStore((s) => s.setGlobalChatOpen)
  const chatUnreadCount = useHQStore((s) => s.chatUnreadCount)
  useNeedsYouCount()
  const systemNotice = useHQStore((s) => s.systemNotice)
  const setSystemNotice = useHQStore((s) => s.setSystemNotice)
  const [isOnline, setIsOnline] = useState(typeof navigator !== 'undefined' ? navigator.onLine : true)
  const [swUpdateAvailable, setSwUpdateAvailable] = useState(false)

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && (e.key.toLowerCase() === 'k' || e.key.toLowerCase() === 'h')) {
        e.preventDefault()
        setGlobalChatOpen(true)
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [setGlobalChatOpen])

  useEffect(() => {
    const goOnline = () => setIsOnline(true)
    const goOffline = () => setIsOnline(false)
    window.addEventListener('online', goOnline)
    window.addEventListener('offline', goOffline)
    return () => {
      window.removeEventListener('online', goOnline)
      window.removeEventListener('offline', goOffline)
    }
  }, [])

  useEffect(() => {
    if (typeof window === 'undefined' || !('serviceWorker' in navigator)) return

    navigator.serviceWorker
      .register('/sw.js')
      .then((reg) => {
        if (reg.waiting) setSwUpdateAvailable(true)
        reg.addEventListener('updatefound', () => {
          const nextWorker = reg.installing
          if (!nextWorker) return
          nextWorker.addEventListener('statechange', () => {
            if (nextWorker.state === 'installed' && navigator.serviceWorker.controller) {
              setSwUpdateAvailable(true)
            }
          })
        })
      })
      .catch((err) => console.warn('[hq-sw] registration failed:', err))

    // Chunks this page loaded before the worker took control: hand them over for offline use.
    navigator.serviceWorker.ready.then((reg) => window.setTimeout(() => {
      const urls = performance
        .getEntriesByType('resource')
        .map((e) => new URL(e.name))
        .filter((u) => u.origin === location.origin && u.pathname.startsWith('/assets/'))
        .map((u) => u.pathname)
      reg.active?.postMessage({ type: 'CACHE_ASSETS', urls })
    }, ROUTE_CHUNKS_SETTLE_MS))
  }, [])

  useEffect(() => {
    if (!systemNotice) return
    const timer = window.setTimeout(() => setSystemNotice(null), SYSTEM_NOTICE_DISMISS_MS)
    return () => window.clearTimeout(timer)
  }, [systemNotice, setSystemNotice])

  const handleSwUpdate = async () => {
    const reg = await navigator.serviceWorker.ready
    reg.waiting?.postMessage({ type: 'SKIP_WAITING' })
    setSwUpdateAvailable(false)
    window.location.reload()
  }

  return (
    <div
      className="flex flex-row fixed inset-0 overflow-hidden"
      style={{
        background: 'var(--bg-base)',
        color: 'var(--text-primary)',
        paddingTop: 'var(--safe-top)',
      }}
    >
      <div className="hq-ambient-bg" />
      <AppRail />
      <div className="flex flex-col flex-1 min-w-0 min-h-0 relative z-10">
      <InstallPrompt />

      {systemNotice && (
        <div
          className="flex items-center justify-between gap-3 px-4 py-2 text-sm relative z-10"
          style={{ background: 'rgba(255,176,0,0.08)', borderBottom: '1px solid rgba(255,176,0,0.2)' }}
        >
          <span className="text-xs font-mono whitespace-pre-line" style={{ color: 'var(--accent-amber)' }}>
            {systemNotice}
          </span>
          <button
            onClick={() => setSystemNotice(null)}
            className="px-2 py-1 rounded-lg text-xs font-mono flex-shrink-0"
            style={{ color: 'var(--text-dim)' }}
          >
            Dismiss
          </button>
        </div>
      )}

      {swUpdateAvailable && (
        <div
          className="flex items-center justify-between px-4 py-2 text-sm relative z-10"
          style={{ background: 'rgba(0,255,163,0.08)', borderBottom: '1px solid rgba(0,255,163,0.2)' }}
        >
          <span className="text-xs font-mono" style={{ color: 'var(--accent-green)' }}>
            A new vault app version is available
          </span>
          <button
            onClick={handleSwUpdate}
            className="px-3 py-1 rounded-lg text-xs font-mono font-bold transition-all"
            style={{ background: 'var(--accent-green)', color: '#000' }}
          >
            Update now
          </button>
        </div>
      )}

      <header
        className="flex-shrink-0 flex items-center gap-2 px-3 sticky top-0 z-30 hq-header"
        style={{ height: '48px' }}
      >
        <Link
          to="/vault"
          className="flex md:hidden items-center gap-2 flex-shrink-0 transition-opacity active:opacity-60"
        >
          <img src="/icons/hq-mark-96.png" alt="Agent HQ" width="28" height="28" className="flex-shrink-0 object-contain" />
          <span className="text-[11px] font-mono font-bold tracking-widest uppercase" style={{ color: 'var(--text-dim)' }}>
            HQ Vault
          </span>
        </Link>

        <div className="flex-1" />

        <button
          type="button"
          onClick={() => window.dispatchEvent(new Event('hq:open-search'))}
          className="p-1.5 rounded-lg text-neutral-400 hover:bg-white/5 transition-all"
          title="Search vault (Cmd+/)"
          aria-label="Search vault"
        >
          <Search className="w-4 h-4" />
        </button>

        <button
          type="button"
          onClick={() => setGlobalChatOpen(true)}
          className="relative p-1.5 rounded-lg text-neutral-400 hover:text-emerald-400 hover:bg-white/5 transition-all flex items-center gap-1.5 font-mono text-xs"
          title="Open chat (Cmd+K)"
        >
          <MessageSquare className="w-4 h-4 text-emerald-400" />
          <span className="hidden sm:inline text-[10px]">Overlay</span>
          {chatUnreadCount > 0 && (
            <span className="absolute -top-1 -right-1 w-4 h-4 rounded-full bg-emerald-400 text-black font-bold text-[9px] flex items-center justify-center animate-bounce">
              {chatUnreadCount}
            </span>
          )}
        </button>

        <CopilotChip />

        <Link
          to="/settings"
          className="md:hidden p-1.5 rounded-lg text-neutral-400 hover:text-neutral-100 hover:bg-white/5 transition-all"
          title="Settings"
          aria-label="Settings"
          activeProps={{ className: 'text-neutral-100 bg-white/5' }}
        >
          <Settings className="w-4 h-4" />
        </Link>

        <div className="flex items-center gap-1.5 flex-shrink-0" title={wsConnected ? 'Connected' : 'Showing data saved on this device'}>
          <div className="relative">
            <div className={`status-dot ${wsConnected ? 'active' : 'error'}`} />
            {wsConnected && (
              <div
                className="absolute inset-0 rounded-full animate-ping"
                style={{ background: 'var(--accent-green)', opacity: 0.25, animationDuration: '3s' }}
              />
            )}
          </div>
          <span
            suppressHydrationWarning
            className="text-[9px] font-mono"
            style={{ color: wsConnected ? 'var(--accent-green)' : isOnline ? 'var(--accent-amber)' : 'var(--accent-red)' }}
          >
            {wsConnected ? 'live' : <SavedDataLabel reconnecting={isOnline} />}
          </span>
        </div>
      </header>

      <main className="flex-1 min-h-0 overflow-hidden relative main-content md:mx-3 md:mb-3 md:rounded-2xl hq-glass-pane">
        <Outlet />
      </main>

      </div>
      <BottomNav />
    </div>
  )
}

const MINUTE_MS = 60_000
// Lets the first route's lazy chunks finish loading before they are reported to the worker.
const ROUTE_CHUNKS_SETTLE_MS = 3000

/** Shown while disconnected: screens are showing data saved on this device. */
function SavedDataLabel({ reconnecting }: { reconnecting: boolean }) {
  const [, tick] = useState(0)
  useEffect(() => {
    const id = window.setInterval(() => tick((n) => n + 1), MINUTE_MS)
    return () => window.clearInterval(id)
  }, [])
  const synced = lastSyncedAt()
  const age = synced ? ` · ${relTime(synced)}` : ''
  return <>{reconnecting ? 'saved' : 'offline'}{age}</>
}

function RootDocument({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <head>
        <HeadContent />
      </head>
      <body>
        <PersistQueryClientProvider
          client={queryClient}
          persistOptions={{ persister: queryPersister, maxAge: CACHE_MAX_AGE_MS, buster: CACHE_BUSTER }}
        >
          {children}
          <Scripts />
        </PersistQueryClientProvider>
      </body>
    </html>
  )
}

