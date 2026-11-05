// The phone app shows some pages (Usage, Board, Timeline) inside its own screen, with its own bar,
// tabs and search. It opens them with `?embed=1` (or `?host=android`, which shipped phones still send);
// the page then hides the web app's chrome. One flag, kept for the tab's life because in-app
// navigation drops the query string.
export const EMBED_PARAM = 'embed'
export const EMBED_ON = '1'
export const LEGACY_HOST_PARAM = 'host'
export const LEGACY_HOST_ANDROID = 'android'
export const EMBED_STORAGE_KEY = 'hq-embed'

/** True for a query string that asks for embedded mode, or for a tab that was already opened that way. */
export function isEmbedRequest(search: string, stored: string | null): boolean {
  const q = new URLSearchParams(search)
  if (q.get(EMBED_PARAM) === EMBED_ON || q.get(LEGACY_HOST_PARAM) === LEGACY_HOST_ANDROID) return true
  return stored === EMBED_ON
}

/** Reads and remembers the flag. False where there is no window (prerender) or storage is blocked. */
export function readEmbedFlag(): boolean {
  if (typeof window === 'undefined') return false
  let stored: string | null = null
  try {
    stored = window.sessionStorage.getItem(EMBED_STORAGE_KEY)
  } catch {
    // storage blocked: the query string alone decides
  }
  const on = isEmbedRequest(window.location.search, stored)
  if (on && stored !== EMBED_ON) {
    try {
      window.sessionStorage.setItem(EMBED_STORAGE_KEY, EMBED_ON)
    } catch {
      // not remembered: the next page load needs the query string again
    }
  }
  return on
}

/** Runs before first paint (head script) so the web chrome never shows, even for a frame. */
export const EMBED_HEAD_SCRIPT =
  "try{var q=new URLSearchParams(location.search);if(q.get('embed')==='1'||q.get('host')==='android')sessionStorage.setItem('hq-embed','1');if(sessionStorage.getItem('hq-embed')==='1')document.documentElement.setAttribute('data-embed','1')}catch(e){}"
