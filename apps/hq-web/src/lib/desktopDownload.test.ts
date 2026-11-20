import { expect, test } from 'bun:test'
import { t } from '~/strings'
import { NOTICE_MS, useToasts } from './toast'
import {
  DESKTOP_DOWNLOAD_EVENT,
  desktopDownloadText,
  handleDesktopDownload,
  listenForDesktopDownloads,
  parseDesktopDownload,
} from './desktopDownload'

const event = (detail: unknown) => ({ detail }) as unknown as Event

function recorder() {
  const seen: Array<['notice' | 'alert', string]> = []
  return { seen, show: { notice: (x: string) => seen.push(['notice', x]), alert: (x: string) => seen.push(['alert', x]) } }
}

test('a saved file says its name and the real path', () => {
  const { seen, show } = recorder()
  handleDesktopDownload(event({ name: 'Plan.xlsx', path: '/srv/example/Downloads/Plan.xlsx', success: true }), show)
  expect(seen).toEqual([['notice', 'Saved to Downloads: Plan.xlsx']])
})

test('a failure is an alert, with or without a name', () => {
  const { seen, show } = recorder()
  handleDesktopDownload(event({ name: 'Plan.xlsx', path: '', success: false }), show)
  handleDesktopDownload(event({ name: '', path: '', success: false }), show)
  expect(seen).toEqual([
    ['alert', t('note_export_save_failed', { name: 'Plan.xlsx' })],
    ['alert', t('note_export_save_failed_unnamed')],
  ])
})

test('a success with no name is reported as a failure, never as a saved file', () => {
  expect(desktopDownloadText({ name: '', path: '/x', success: true }).ok).toBe(false)
})

test('a detail that is not a download result shows nothing', () => {
  const { seen, show } = recorder()
  for (const detail of [null, undefined, 'x', 3, {}, { success: 'yes' }]) handleDesktopDownload(event(detail), show)
  expect(seen).toEqual([])
  expect(parseDesktopDownload({ success: true, name: 5 })).toEqual({ name: '', path: '', success: true })
})

test('the listener hears the event by name and stops when told', () => {
  const handlers = new Map<string, EventListener>()
  const target = {
    addEventListener: (type: string, fn: EventListener) => void handlers.set(type, fn),
    removeEventListener: (type: string) => void handlers.delete(type),
  }
  const stop = listenForDesktopDownloads(target)
  expect([...handlers.keys()]).toEqual([DESKTOP_DOWNLOAD_EVENT])
  expect(DESKTOP_DOWNLOAD_EVENT).toBe('hq-desktop-download')
  stop()
  expect(handlers.size).toBe(0)
})

test('a download event on the page reaches the real toast store as a plain notice that goes away in 6 s', () => {
  const handlers = new Map<string, EventListener>()
  const target = {
    addEventListener: (type: string, fn: EventListener) => void handlers.set(type, fn),
    removeEventListener: (type: string) => void handlers.delete(type),
  }
  useToasts.setState({ toasts: [] })
  listenForDesktopDownloads(target)
  handlers.get(DESKTOP_DOWNLOAD_EVENT)!(event({ name: 'Matooke.html', path: '/srv/example/Downloads/Matooke.html', success: true }))
  handlers.get(DESKTOP_DOWNLOAD_EVENT)!(event({ name: 'Matooke.xlsx', path: '', success: false }))
  const [ok, bad] = useToasts.getState().toasts
  expect(ok).toMatchObject({ text: 'Saved to Downloads: Matooke.html', kind: 'status' })
  expect(ok!.undo).toBeUndefined()
  expect(bad).toMatchObject({ kind: 'alert' })
  expect(NOTICE_MS).toBe(6000)
  useToasts.setState({ toasts: [] })
})
