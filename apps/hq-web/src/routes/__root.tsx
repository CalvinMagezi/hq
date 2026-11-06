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
        {/* The app has its own search and quick note screens. */}
        {!embedded && (
          <>
            <SearchOverlay />
            <QuickNoteOverlay />
          </>
        )}
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
  if (state !== 'connected' || !chip.label) return null
  return (
    <Link to={chip.to} className={`a-bal ${chip.limit ? 'warn' : ''}`} title={chip.title} aria-label={t('balance_aria', { balance: chip.label })}>
      <BalanceChip state={state} label={chip.label} title={chip.title} limit={chip.limit} bare />
    </Link>
  )
}

/** Search, the balance and Settings in one menu below 360 px (the design's `.a-top-more`). */
function MoreMenu({ state, chip, lite }: { state: ConnectionState; chip: ReturnType<typeof useChip>; lite: boolean }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="a-top-more">
      <button type="button" className="a-ib" aria-label={t('more_menu')} aria-expanded={open} onClick={() => setOpen(!open)}>
        <MoreHorizontal className="a-ic" aria-hidden="true" />
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

/** The search field from 1024 px, the search icon below it (the design's `.a-find`). */
function FindButton({ lite }: { lite: boolean }) {
  const words = lite ? t('search_field_lite') : t('search_field_all')
  return (
    <button type="button" className="a-find" aria-label={words} onClick={() => window.dispatchEvent(new Event('hq:open-search'))}>
      <Search className="a-ic" aria-hidden="true" />
      <span>{words}</span>
      <kbd className="a-kbd">{t('search_kbd')}</kbd>
    </button>
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
  const workingHelpers = useHQStore((s) => s.workingHelpersCount)
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

  // "Send to Chat" from anywhere: the text is already the draft, Chat opens with it.
  useEffect(() => {
    if (lite) return
    const open = () => void navigate({ to: '/chat' })
    window.addEventListener('hq:open-chat', open)
    return () => window.removeEventListener('hq:open-chat', open)
  }, [navigate, lite])

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
      <div className="a-app hq-root">
        <ToastHost />
        <SignInPrompt />
        <main className="a-main">
          <div className="hq-place">
            {signInNeeded && !ownsSignedOut(pathname) ? <SignedOutView pathname={pathname} embedded={false} /> : <Outlet />}
          </div>
        </main>
      </div>
    )
  }

  const signedOut = connection.state === 'signed_out'
  return (
    <div className="a-app hq-root">
      {!embedded && (
        <a className="a-skip" href="#hq-main">
          {t('skip_to_content')}
        </a>
      )}
      {!embedded && (
        <>
          <InstallPrompt />
          <SignInPrompt />
          <ConnectionSheet connection={connection} open={sheetOpen} onClose={() => setSheetOpen(false)} triggerRef={statusLineRef} />
          <ToastHost />
        </>
      )}
      {/* One sheet for every note link, whatever page it was clicked in; inside .a-app so it takes the design's widths. */}
      <VaultNoteDrawer />
      <div className="a-shell">
        {/* Signed out, only Sign in shows: no places, no gear (FirstRun 2). */}
        {!embedded && !signedOut && <AppRail />}
        {!embedded && (
          <header className="a-top">
            <HeaderTitle
              title={placeTitle(pathname)}
              connection={connection}
              fact={placeFact(pathname, { activeTasks, workingHelpers })}
              onClick={() => setSheetOpen(true)}
              buttonRef={statusLineRef}
            />
            <div className="a-top-actions">
              {signedOut ? (
                <button type="button" className="a-btn compact" onClick={openSignIn}>
                  {t('signed_out_button')}
                </button>
              ) : (
                <>
                  <FindButton lite={lite} />
                  {!lite && <CopilotChip state={connection.state} chip={chip} />}
                  <Link to="/settings" className="a-ib a-top-gear" aria-label={t('place_settings')}>
                    <Settings className="a-ic" aria-hidden="true" />
                  </Link>
                  <MoreMenu state={connection.state} chip={chip} lite={lite} />
                </>
              )}
            </div>
          </header>
        )}
        <main className="a-main" id="hq-main">
          {!embedded && <SavedStrip state={connection.state} pathname={pathname} />}
          <div className="hq-place">
            {signInNeeded ? <SignedOutView pathname={pathname} embedded={embedded} /> : <Outlet />}
          </div>
        </main>
        {!embedded && !signedOut && <BottomNav />}
      </div>
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

