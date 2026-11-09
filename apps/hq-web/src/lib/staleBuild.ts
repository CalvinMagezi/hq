// A page left open across an update still names the old build's code files, which are gone.
// Opening a screen it has not loaded yet then fails. One reload picks up the new build.

const RELOAD_KEY = 'hq-stale-build-reload'
/** Long enough that a build which really is broken shows its error instead of reloading forever. */
const RELOAD_GUARD_MS = 30_000

// Safari, Chrome and Firefox each word a failed dynamic import differently.
const STALE_BUILD_MESSAGES = [
  /importing a module script failed/i,
  /failed to fetch dynamically imported module/i,
  /error loading dynamically imported module/i,
  /unable to preload css/i,
]

export function isStaleBuildError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error ?? '')
  return STALE_BUILD_MESSAGES.some((pattern) => pattern.test(message))
}

type GuardStore = Pick<Storage, 'getItem' | 'setItem'>

/** Reloads unless it just did. False means the caller should show the error. */
export function reloadOnceForNewBuild(store: GuardStore, now: number, reload: () => void): boolean {
  try {
    const last = Number(store.getItem(RELOAD_KEY) ?? 0)
    if (now - last < RELOAD_GUARD_MS) return false
    store.setItem(RELOAD_KEY, String(now))
  } catch {
    // Without the guard a reload could repeat forever, so the error is shown instead.
    return false
  }
  reload()
  return true
}

export function tryNewBuildReload(): boolean {
  return reloadOnceForNewBuild(window.sessionStorage, Date.now(), () => window.location.reload())
}

/** Vite reports a failed code-file load here before the route ever sees it. */
export function installStaleBuildRecovery(): void {
  window.addEventListener('vite:preloadError', (event) => {
    if (tryNewBuildReload()) event.preventDefault()
  })
}
