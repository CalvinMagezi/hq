// The Android app shows some pages (Home, Usage, Board, Timeline) inside its own screen, with its own
// bar, tabs and search. It opens them with `?host=android`; the page then hides the web app's chrome.
// The flag is kept for the tab's life because in-app navigation drops the query string.
export const EMBED_PARAM = 'host'
export const EMBED_ANDROID = 'android'
export const EMBED_STORAGE_KEY = 'hq-embed-host'

/** True for a query string that asks for the Android host, or for a tab that was already opened that way. */
export function isEmbedRequest(search: string, stored: string | null): boolean {
  if (new URLSearchParams(search).get(EMBED_PARAM) === EMBED_ANDROID) return true
  return stored === EMBED_ANDROID
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
  if (on && stored !== EMBED_ANDROID) {
    try {
      window.sessionStorage.setItem(EMBED_STORAGE_KEY, EMBED_ANDROID)
    } catch {
      // not remembered: the next page load needs the query string again
    }
  }
  return on
}

/** Runs before first paint (head script) so the web chrome never shows, even for a frame. */
export const EMBED_HEAD_SCRIPT =
  "try{var q=new URLSearchParams(location.search).get('host');if(q==='android')sessionStorage.setItem('hq-embed-host','android');if(sessionStorage.getItem('hq-embed-host')==='android')document.documentElement.setAttribute('data-host','android')}catch(e){}"
