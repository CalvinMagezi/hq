// The macOS app saves a file the page downloads into Downloads and then dispatches this window
// event with how it went. The page cannot call the app, so this event is the whole channel.

import { t } from '~/strings'
import { toastAlert, toastNotice } from './toast'

export const DESKTOP_DOWNLOAD_EVENT = 'hq-desktop-download'

export interface DesktopDownload {
  name: string
  path: string
  success: boolean
}

/** The event's detail as a download result, or null when it is not shaped like one. */
export function parseDesktopDownload(detail: unknown): DesktopDownload | null {
  if (typeof detail !== 'object' || detail === null) return null
  const d = detail as Record<string, unknown>
  if (typeof d.success !== 'boolean') return null
  return {
    name: typeof d.name === 'string' ? d.name : '',
    path: typeof d.path === 'string' ? d.path : '',
    success: d.success,
  }
}

/** The toast text for one finished download: the file name on success, a plain failure otherwise. The shell offers no way to reveal the file, so none is promised. */
export function desktopDownloadText(result: DesktopDownload): { text: string; ok: boolean } {
  if (result.success && result.name) {
    return { text: t('note_export_saved', { name: result.name }), ok: true }
  }
  const text = result.name ? t('note_export_save_failed', { name: result.name }) : t('note_export_save_failed_unnamed')
  return { text, ok: false }
}

/** Shows the toast for one event; ignores an event that is not a download result. */
export function handleDesktopDownload(
  event: Event,
  show: { notice: (text: string) => void; alert: (text: string) => void } = { notice: toastNotice, alert: toastAlert },
): void {
  const result = parseDesktopDownload((event as CustomEvent).detail)
  if (!result) return
  const { text, ok } = desktopDownloadText(result)
  if (ok) show.notice(text)
  else show.alert(text)
}

/** Starts listening on the window; returns the function that stops. */
export function listenForDesktopDownloads(target: Pick<Window, 'addEventListener' | 'removeEventListener'> = window): () => void {
  const on = (event: Event) => handleDesktopDownload(event)
  target.addEventListener(DESKTOP_DOWNLOAD_EVENT, on)
  return () => target.removeEventListener(DESKTOP_DOWNLOAD_EVENT, on)
}
