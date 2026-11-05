import {
  HeadContent,
  Link,
  Outlet,
  useNavigate,
  useRouterState,
  Scripts,
  createRootRoute,
  redirect,
} from '@tanstack/react-router'
import { PersistQueryClientProvider } from '@tanstack/react-query-persist-client'
import { CACHE_BUSTER, CACHE_MAX_AGE_MS, queryClient, queryPersister } from '~/lib/offlineCache'
import type { ReactNode } from 'react'
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { MoreHorizontal, Search, Settings } from 'lucide-react'
import { useHQStore } from '~/store/hqStore'
import { WebSocketProvider } from '~/context/WebSocketContext'
import { SearchOverlay } from '~/components/SearchOverlay'
import { QuickNoteOverlay } from '~/components/QuickNoteOverlay'
import { GlobalChatOverlay } from '~/components/chat/GlobalChatOverlay'
import { VaultNoteDrawer } from '~/components/VaultNoteDrawer'
import { InstallPrompt } from '~/components/InstallPrompt'
import { openSignIn, SignedOutView, SignInPrompt } from '~/components/SignInPrompt'
import { isCover, needsSignInPage, ownsSignedOut } from '~/lib/covers'
import { BalanceChip, HeaderTitle, placeFact, placeTitle } from '~/components/StatusLine'
import { SavedStrip } from '~/components/SavedStrip'
import { ConnectionSheet } from '~/components/ConnectionSheet'
import { useConnection, type ConnectionState } from '~/lib/connection'
import { ToastHost } from '~/components/ui/Toast'
import { AppRail } from '~/components/AppRail'
import { BottomNav } from '~/components/BottomNav'
import { useNeedsYouCount } from '~/components/sessions/useNeedsYouCount'
import alignedCss from '~/styles/aligned-ui.css?url'
import appCss from '../../app.css?url'
import { t } from '~/strings'
import { startDataCounter } from '~/lib/dataUsage'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { visibleTimer } from '~/lib/visibleTimer'
import { openrouterChipLabel, openrouterChipTitle, type OpenRouterUsageResponse } from '~/lib/openrouterUsageApi'
import { budgetChipLabel, budgetChipTitle, type BudgetsResponse } from '~/lib/usageApi'
import { chipLabel, chipTitle, type CopilotUsage } from '~/lib/copilotUsageApi'
import { hasWebToken, hqJson, needsSignIn, onSignInChange, refuseSignIn, signedOutElsewhere, takeSignInArrival, wasSignedOut } from '~/lib/hqAuth'
import { lastKnownEdition, setEdition, useEdition } from '~/lib/edition'
import { fetchHello } from '~/lib/hello'
import { noteHelloVersion, noteWaitingWorker } from '~/lib/version'
import { noteHqTimeZone } from '~/lib/time'
import { legacyRedirect, liteRedirect } from '~/lib/routes'
import { toastNotice } from '~/lib/toast'
import { isSharedComputer, watchIdle } from '~/lib/sharedComputer'
import { listenForWipe } from '~/lib/wipe'
import { syncPushSubscription } from '~/lib/webPush'
import { fetchSetupStatus, shouldRedirectToSetup, showsNoSignIn } from '~/lib/setupApi'
import { EMBED_HEAD_SCRIPT, readEmbedFlag } from '~/lib/embedHost'


export const Route = createRootRoute({
  // Every old address (and `/`) goes to its new home before anything draws, keeping the item and the fragment.
  beforeLoad: ({ location }) => {
    const target = legacyRedirect(
      { pathname: location.pathname, search: location.searchStr ?? '', hash: location.hash ?? '' },
      { edition: lastKnownEdition(), signInLink: takeSignInArrival() },
    )
    if (target) throw redirect({ to: target.to as '/', search: target.search as never, hash: target.hash, replace: true })
  },
  head: () => ({
    scripts: [{ children: EMBED_HEAD_SCRIPT }],
    meta: [
      { charSet: 'utf-8' },
      { name: 'viewport', content: 'width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content' },
      { title: t('doc_title') },
      { name: 'theme-color', content: '#000000' },
      { name: 'apple-mobile-web-app-capable', content: 'yes' },
      { name: 'apple-mobile-web-app-status-bar-style', content: 'black-translucent' },
    ],
    links: [
      // The design system's classes (a-*), before app.css so today's places keep their look.
      { rel: 'stylesheet', href: alignedCss },
      { rel: 'stylesheet', href: appCss },
      { rel: 'manifest', href: '/manifest.json' },
      { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/icons/hq-favicon-32.png' },
      { rel: 'icon', type: 'image/png', sizes: '192x192', href: '/icons/hq-icon-192.png' },
      { rel: 'apple-touch-icon', sizes: '180x180', href: '/icons/hq-apple-touch-180.png' },
    ],
  }),
  component: RootComponent,
})

/** True inside the Android app's own screen (`?host=android`). The server render and the first client render say false, so hydration matches. */
/** How often an open app asks whether a new build was installed on the server. */
const SW_UPDATE_CHECK_MS = 30 * 60 * 1000

function useEmbedded(): boolean {
  return useSyncExternalStore(
    () => () => undefined,
    readEmbedFlag,
    () => false,
  )
}

function RootComponent() {
  const embedded = useEmbedded()
  return (
    <RootDocument>
      <WebSocketProvider>
        <SetupRedirect />
        <VaultShell embedded={embedded} />
        {/* The app has its own search, quick note and chat screens. */}
        {!embedded && (
          <>
            <SearchOverlay />
            <QuickNoteOverlay />
            <GlobalChatOverlay />
          </>
        )}
        {/* One drawer for every note link, whatever page or overlay it was clicked in. It stays in the app's host, where a note link in a task would otherwise do nothing. */}
        <VaultNoteDrawer />
      </WebSocketProvider>
    </RootDocument>
  )
}

function SetupRedirect() {
  // An effect, not useQuery, for the same prerender reason as CopilotChip. Checked once per load.
  const navigate = useNavigate()
  useEffect(() => {
    if (window.location.pathname === '/setup') return
    // The Android app has its own first-run screens; its host shows the page it asked for.
    if (readEmbedFlag()) return
    fetchSetupStatus()
      .then((status) => {
        // Replaces the page it left, so Back from setup does not return to a page that sends the visitor straight back.
        if (shouldRedirectToSetup(status) || showsNoSignIn(status)) void navigate({ to: '/setup', replace: true })
      })
      .catch(() => undefined)
  }, [navigate])
  return null
}

/** The balance or limit chip's words and its tone, or null when there is nothing to say. */
function useChip() {
  // A plain effect, not useQuery: a query hook in the root shell kept the build's prerender from
  // exiting, and effects never run there.
  const [data, setData] = useState<CopilotUsage | undefined>(undefined)
  const [openrouter, setOpenrouter] = useState<OpenRouterUsageResponse | undefined>(undefined)
  const [budgets, setBudgets] = useState<BudgetsResponse | undefined>(undefined)
  const load = useCallback(() => {
    hqJson<CopilotUsage>('/api/copilot-usage')
      .then(setData)
      .catch(() => undefined)
    hqJson<OpenRouterUsageResponse>('/api/openrouter-usage')
      .then(setOpenrouter)
      .catch(() => undefined)
    hqJson<BudgetsResponse>('/api/budgets')
      .then(setBudgets)
      .catch(() => undefined)
  }, [])
  // No timer: HQ says when a limit or a key changed, and the chip catches up when the tab is shown.
  useEffect(() => {
    load()
    const onVisible = () => document.visibilityState === 'visible' && load()
    document.addEventListener('visibilitychange', onVisible)
    return () => document.removeEventListener('visibilitychange', onVisible)
  }, [load])
  useRefreshOn(['budget:change', 'settings:change'], load)
  const copilotLabel = chipLabel(data)
  // A budget that needs attention outranks a balance: it is the thing about to block a call.
  const budgetLabel = budgetChipLabel(budgets)
  const label = budgetLabel ?? copilotLabel ?? openrouterChipLabel(openrouter)
  const title = budgetLabel ? budgetChipTitle(budgets) : copilotLabel ? chipTitle(data) : openrouterChipTitle(openrouter)
  return { label, title, limit: Boolean(budgetLabel), to: '/settings/spending' as const }
}

function CopilotChip({ state, chip }: { state: ConnectionState; chip: ReturnType<typeof useChip> }) {
  return (
    <Link to={chip.to} className="hq-chip-link">
      <BalanceChip state={state} label={chip.label} title={chip.title} limit={chip.limit} />
    </Link>
  )
}

/** Search, the balance and Settings in one menu on the narrowest phones. */
function MoreMenu({ state, chip, lite }: { state: ConnectionState; chip: ReturnType<typeof useChip>; lite: boolean }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="hq-more">
      <button type="button" className="a-ib" aria-label={t('more_menu')} aria-expanded={open} onClick={() => setOpen(!open)}>
        <MoreHorizontal className="w-4 h-4" />
      </button>
      {open && (
        <div className="a-menu hq-more-menu" role="menu">
          <button type="button" role="menuitem" className="a-menu-item" onClick={() => { setOpen(false); window.dispatchEvent(new Event('hq:open-search')) }}>
            {t('more_search')}
          </button>
          {!lite && state === 'connected' && chip.label && (
            <Link to={chip.to} role="menuitem" className="a-menu-item" onClick={() => setOpen(false)}>
              {t('more_spending', { balance: chip.label })}
            </Link>
          )}
          <Link to="/settings" role="menuitem" className="a-menu-item" onClick={() => setOpen(false)}>
            {t('place_settings')}
          </Link>
        </div>
      )}
    </div>
  )
}

const BUDGET_QUERY_KEYS = ['usage-budgets', 'usage-forecast', 'usage-providers', 'copilot-usage', 'openrouter-usage']
const refetchBudgetQueries = () => {
  for (const queryKey of BUDGET_QUERY_KEYS) void queryClient.invalidateQueries({ queryKey: [queryKey] })
}

function VaultShell({ embedded }: { embedded: boolean }) {
  useEffect(() => startDataCounter(), [])
  useRefreshOn(['budget:change', 'settings:change'], refetchBudgetQueries)
  const wsConnected = useHQStore((s) => s.wsConnected)
  const setGlobalChatOpen = useHQStore((s) => s.setGlobalChatOpen)
  useNeedsYouCount()
  const signInNeeded = useSyncExternalStore(onSignInChange, needsSignIn, () => false)
  const pathname = useRouterState({ select: (r) => r.location.pathname })
  const [sheetOpen, setSheetOpen] = useState(false)
  const connection = useConnection()
  const chip = useChip()
  const edition = useEdition()
  const lite = edition === 'lite'
  const navigate = useNavigate()
  const activeTasks = useHQStore((s) => s.tasks.filter((x) => x.status === 'in_progress').length)
  const statusLineRef = useRef<HTMLButtonElement>(null)
  const cover = isCover(pathname)
  const signInPage = needsSignInPage({ signInNeeded, hadSignIn: hasWebToken() || wasSignedOut() })

  // A browser that never signed in opens the full page; once signed in, /signin returns to the start place.
  useEffect(() => {
    if (embedded) return
    if (signInPage && !ownsSignedOut(pathname)) void navigate({ to: '/signin', replace: true })
    else if (!signInNeeded && pathname === '/signin') void navigate({ to: lite ? '/notes' : '/chat', replace: true })
  }, [embedded, signInPage, signInNeeded, pathname, lite, navigate])

  // HQ says which edition it is; the rail and bar draw the places that edition serves.
  useEffect(() => {
    let alive = true
    const look = () =>
      void fetchHello().then((h) => {
        if (!alive || !h) return
        setEdition(h.edition)
        noteHelloVersion(h)
        noteHqTimeZone(h)
      })
    look()
    // An HQ that updated while the tab sat hidden is noticed the moment the tab is shown.
    const onVisible = () => document.visibilityState === 'visible' && look()
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      alive = false
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [])

  // Lite has no Chat, Helpers or Spending: a stored link opens Notes (or This browser) with a line saying so.
  useEffect(() => {
    if (!lite) return
    const hit = liteRedirect(pathname)
    if (!hit) return
    void navigate({ to: hit.target.to as '/', replace: true })
    if (hit.notice?.key === 'lite_no_place') toastNotice(t('lite_no_place', { place: t(hit.notice.place === 'chat' ? 'place_chat' : 'place_helpers') }))
    if (hit.notice?.key === 'lite_no_spending') toastNotice(t('lite_no_spending'))
  }, [lite, pathname, navigate])

  useEffect(() => {
    document.title = t('document_title', { place: placeTitle(pathname) })
  }, [pathname])

  // Another tab of this HQ was refused and wiped the browser: this one shows nothing either.
  useEffect(() => listenForWipe(signedOutElsewhere), [])
  // A shared computer signs out after 30 minutes with no input, wiping what this tab held.
  useEffect(() => (isSharedComputer() && !signInNeeded ? watchIdle(refuseSignIn) : undefined), [signInNeeded])

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && (e.key.toLowerCase() === 'k' || e.key.toLowerCase() === 'h')) {
        if (lite) return
        e.preventDefault()
        setGlobalChatOpen(true)
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [setGlobalChatOpen, lite])

  useEffect(() => {
    if (typeof window === 'undefined' || !('serviceWorker' in navigator)) return

    // An installed app stays open for days and the browser looks for a new worker only on a
    // navigation, so it is asked again on a timer and whenever the app comes back into view.
    let registration: ServiceWorkerRegistration | undefined
    const checkForUpdate = () => void registration?.update().catch(() => undefined)
    const stopUpdateTimer = visibleTimer(checkForUpdate, SW_UPDATE_CHECK_MS)

    navigator.serviceWorker
      .register('/sw.js')
      .then((reg) => {
        registration = reg
        if (reg.waiting) noteWaitingWorker(true)
        reg.addEventListener('updatefound', () => {
          const nextWorker = reg.installing
          if (!nextWorker) return
          nextWorker.addEventListener('statechange', () => {
            if (nextWorker.state === 'installed' && navigator.serviceWorker.controller) {
              noteWaitingWorker(true)
            }
          })
        })
      })
      .catch((err) => console.warn('[hq-sw] registration failed:', err))

    // A device that turned push on re-sends its subscription, so the server keeps it current.
    void syncPushSubscription()

    // Chunks this page loaded before the worker took control: hand them over for offline use.
    navigator.serviceWorker.ready.then((reg) => window.setTimeout(() => {
      const urls = performance
        .getEntriesByType('resource')
        .map((e) => new URL(e.name))
        .filter((u) => u.origin === location.origin && u.pathname.startsWith('/assets/'))
        .map((u) => u.pathname)
      reg.active?.postMessage({ type: 'CACHE_ASSETS', urls })
    }, ROUTE_CHUNKS_SETTLE_MS))
    return () => {
      stopUpdateTimer()
    }
  }, [])

  if (cover && !embedded) {
    return (
      <div className="fixed inset-0 overflow-hidden" style={{ background: 'var(--bg-base)', color: 'var(--text-primary)', paddingTop: 'var(--safe-top)' }}>
        <ToastHost />
        <SignInPrompt />
        <main className="h-full overflow-hidden">
          {signInNeeded && !ownsSignedOut(pathname) ? <SignedOutView pathname={pathname} embedded={false} /> : <Outlet />}
        </main>
      </div>
    )
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
      {!embedded && <AppRail />}
      <div className="flex flex-col flex-1 min-w-0 min-h-0 relative z-10">
      {!embedded && (
        <>
          <InstallPrompt />
          <SignInPrompt />
          <ConnectionSheet connection={connection} open={sheetOpen} onClose={() => setSheetOpen(false)} triggerRef={statusLineRef} />
        </>
      )}

      {!embedded && <ToastHost />}

      {!embedded && <header
        className="flex-shrink-0 flex items-center gap-2 px-3 sticky top-0 z-30 hq-header"
        style={{ minHeight: '48px' }}
      >
        <Link
          to="/"
          className="flex md:hidden items-center gap-2 flex-shrink-0 transition-opacity active:opacity-60"
        >
          <img src="/icons/hq-mark-96.png" alt="Agent HQ" width="28" height="28" className="flex-shrink-0 object-contain" />
          {/* Gives way on the narrowest phones so the clock always fits. */}
          <span className="hidden min-[380px]:inline text-xs font-mono font-bold tracking-widest uppercase" style={{ color: 'var(--text-dim)' }}>
            HQ Vault
          </span>
        </Link>

        <HeaderTitle
          title={placeTitle(pathname)}
          connection={connection}
          fact={placeFact(pathname, { activeTasks, workingHelpers: 0 })}
          onClick={() => setSheetOpen(true)}
          buttonRef={statusLineRef}
        />

        <button
          type="button"
          onClick={() => window.dispatchEvent(new Event('hq:open-search'))}
          className="hq-header-search p-1.5 rounded-lg text-neutral-400 hover:bg-white/5 transition-all"
          title="Search vault (Cmd+/)"
          aria-label="Search vault"
        >
          <Search className="w-4 h-4" />
        </button>

        {!lite && <CopilotChip state={connection.state} chip={chip} />}

        <Link
          to="/settings"
          className="hq-header-gear md:hidden p-1.5 rounded-lg text-neutral-400 hover:text-neutral-100 hover:bg-white/5 transition-all"
          title="Settings"
          aria-label="Settings"
          activeProps={{ className: 'text-neutral-100 bg-white/5' }}
        >
          <Settings className="w-4 h-4" />
        </Link>

        {connection.state === 'signed_out' && (
          <button type="button" className="a-btn compact" onClick={openSignIn}>
            {t('signed_out_button')}
          </button>
        )}

        <MoreMenu state={connection.state} chip={chip} lite={lite} />
      </header>}

      {!embedded && <SavedStrip state={connection.state} pathname={pathname} />}

      <main className="flex-1 min-h-0 overflow-hidden relative main-content md:mx-3 md:mb-3 md:rounded-2xl hq-glass-pane">
        {signInNeeded ? <SignedOutView pathname={pathname} embedded={embedded} /> : <Outlet />}
      </main>

      </div>
      {!embedded && <BottomNav />}
    </div>
  )
}

// Lets the first route's lazy chunks finish loading before they are reported to the worker.
const ROUTE_CHUNKS_SETTLE_MS = 3000

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

