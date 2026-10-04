/// <reference lib="webworker" />
declare const self: ServiceWorkerGlobalScope

// Only the shell cache is per build. Hashed /assets/ files never change under the
// same name, so they live in one stable cache and survive deploys; API copies too.
declare const __BUILD_TIME__: string
const SHELL_CACHE = `shell-hq-${__BUILD_TIME__}`
const STATIC_CACHE = 'hq-static'
const API_CACHE = 'hq-api'
const KEEP_CACHES = [SHELL_CACHE, STATIC_CACHE, API_CACHE]

const SHELL_ASSETS = [
  '/manifest.json',
  '/icons/hq-icon-192.png',
  '/icons/hq-icon-512.png',
  '/icons/hq-mark-96.png',
  '/hq-agent.svg',
  '/notification.wav',
  '/offline.html',
]

// Slow links: give the network this long before answering from the cache.
const NAV_TIMEOUT_MS = 3000
const API_TIMEOUT_MS = 4000
// ponytail: keeps the newest N hashed assets; a per-build manifest if the cache ever needs to be exact.
const STATIC_CACHE_MAX_ENTRIES = 400
const API_CACHE_MAX_ENTRIES = 200
// React Query and zustand already keep the vault, tasks and recent chats in IndexedDB.
// These reads have no such copy, so the worker keeps one for offline use. The inbox
// store holds only the last tab loaded, so each notifications filter is kept here too.
const SW_CACHED_API = [
  /^\/api\/notifications$/,
  /^\/api\/vault-asset$/,
  /^\/api\/search$/,
  /^\/api\/vault\/folders$/,
  /^\/api\/tasks\/[^/]+\/comments$/,
  /^\/api\/threads\/[^/]+\/messages$/,
]
// Marks a response served from this cache, so the page does not count it as a sync.
const FROM_CACHE_HEADER = 'x-hq-cache'
// The server's CORS layer sends `Vary: origin`, and module imports carry an Origin
// header the saved copies lack, so a strict match would miss every cached chunk.
const MATCH: CacheQueryOptions = { ignoreVary: true }

/** Files the app shell loads on start: the JS, CSS and fonts index.html references. */
function referencedAssets(html: string): string[] {
  return [...new Set(html.match(/\/assets\/[\w.-]+\.(?:js|css|woff2)/g) ?? [])]
}

/** Clones synchronously: the page may start reading `res` right after this call. */
async function cacheShell(res: Response): Promise<void> {
  const copy = res.clone()
  const html = await copy.clone().text()
  await (await caches.open(SHELL_CACHE)).put('/', copy)
  const statics = await caches.open(STATIC_CACHE)
  await Promise.all(
    referencedAssets(html).map(async (url) => {
      if (!(await statics.match(url, MATCH))) await statics.add(url)
    }),
  )
}

async function trimCache(name: string, maxEntries: number): Promise<void> {
  const cache = await caches.open(name)
  const keys = await cache.keys()
  const excess = keys.length - maxEntries
  if (excess > 0) await Promise.all(keys.slice(0, excess).map((k) => cache.delete(k)))
}

function markedFromCache(res: Response): Response {
  const headers = new Headers(res.headers)
  headers.set(FROM_CACHE_HEADER, '1')
  return new Response(res.body, { status: res.status, statusText: res.statusText, headers })
}

/** Resolves with the network response, or null once `ms` passes or the fetch fails. */
function networkWithin(request: Request, ms: number): { quick: Promise<Response | null>; full: Promise<Response> } {
  const full = fetch(request)
  const quick = Promise.race([
    full.catch(() => null),
    new Promise<null>((resolve) => setTimeout(() => resolve(null), ms)),
  ])
  return { quick, full }
}

// Install: cache the shell and everything it loads, so the next open works offline.
self.addEventListener('install', (event) => {
  event.waitUntil(
    (async () => {
      await (await caches.open(SHELL_CACHE)).addAll(SHELL_ASSETS)
      const res = await fetch('/', { cache: 'no-cache' })
      if (res.ok) await cacheShell(res)
    })(),
  )
})

// Activate: drop the previous build's shell cache and take over open pages.
self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys()
      await Promise.all(keys.filter((k) => !KEEP_CACHES.includes(k)).map((k) => caches.delete(k)))
      await trimCache(STATIC_CACHE, STATIC_CACHE_MAX_ENTRIES)
      await trimCache(API_CACHE, API_CACHE_MAX_ENTRIES)
      await self.clients.claim()
    })(),
  )
})

self.addEventListener('message', (event) => {
  if (event.data?.type === 'SKIP_WAITING') {
    self.skipWaiting()
  }
  // The first visit loads its chunks before this worker controls the page, so
  // the page reports them and they are saved here for offline use.
  if (event.data?.type === 'CACHE_ASSETS' && Array.isArray(event.data.urls)) {
    event.waitUntil(
      caches.open(STATIC_CACHE).then((cache) =>
        Promise.all(
          (event.data.urls as string[])
            .filter((u) => u.startsWith('/assets/'))
            .map(async (u) => {
              if (!(await cache.match(u, MATCH))) await cache.add(u).catch(() => undefined)
            }),
        ),
      ),
    )
  }
})

// A reply alert opens its chat: in an HQ window already open, or a new one.
self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const target = new URL((event.notification.data?.url as string | undefined) ?? '/chat', self.location.origin).href
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true })
      const open = windows.find((w) => new URL(w.url).origin === self.location.origin)
      if (!open) {
        await self.clients.openWindow(target)
        return
      }
      await open.focus()
      await open.navigate(target).catch(() => undefined)
    })(),
  )
})

self.addEventListener('fetch', (event) => {
  const { request } = event
  const url = new URL(request.url)
  if (request.method !== 'GET' || url.origin !== self.location.origin) return

  // Pages: every route is the same SPA shell. Network first, cached shell after 3s.
  if (request.mode === 'navigate') {
    const { quick, full } = networkWithin(request, NAV_TIMEOUT_MS)
    event.waitUntil(full.then((res) => (res.ok ? cacheShell(res) : undefined)).catch(() => undefined))
    event.respondWith(
      (async () => {
        const res = await quick
        if (res?.ok) return res
        const shell = await caches.match('/', MATCH)
        return shell ?? res ?? (await caches.match('/offline.html', MATCH)) ?? Response.error()
      })(),
    )
    return
  }

  // Those API reads: network first; after 4s or on failure, the last saved copy of any age.
  if (SW_CACHED_API.some((re) => re.test(url.pathname))) {
    const { quick, full } = networkWithin(request, API_TIMEOUT_MS)
    event.waitUntil(
      full
        .then((res) => {
          if (!res.ok) return
          const copy = res.clone()
          return caches
            .open(API_CACHE)
            .then((cache) => cache.put(request, copy))
            .then(() => trimCache(API_CACHE, API_CACHE_MAX_ENTRIES))
        })
        .catch(() => undefined),
    )
    event.respondWith(
      (async () => {
        const res = await quick
        if (res) return res
        const cached = await caches.match(request, { ...MATCH, cacheName: API_CACHE })
        return cached ? markedFromCache(cached) : Response.json({ error: 'offline' }, { status: 503 })
      })(),
    )
    return
  }

  // Hashed build assets never change: cache first.
  if (url.pathname.startsWith('/assets/')) {
    event.respondWith(
      (async () => {
        const cached = await caches.match(request, { ...MATCH, cacheName: STATIC_CACHE })
        if (cached) return cached
        const res = await fetch(request)
        if (res.ok) await (await caches.open(STATIC_CACHE)).put(request, res.clone())
        return res
      })(),
    )
    return
  }

  if (url.pathname.startsWith('/icons/')) {
    event.respondWith(caches.match(request, MATCH).then((cached) => cached ?? fetch(request)))
  }
})
