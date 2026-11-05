import { beforeEach, describe, expect, test } from 'bun:test'
import { HqHttpError, readJson } from './hqAuth'
import { useToasts } from './toast'
import {
  CLIENT_LEVEL,
  failureLine,
  keepUnsaved,
  noteHelloVersion,
  onBeforeReload,
  pageIsOlder,
  reloadForNewer,
  resetVersionForTests,
  takeUnsaved,
} from './version'

const NEWER_TEXT = 'A newer HQ is running. Reload to keep going.'

beforeEach(() => {
  resetVersionForTests()
  useToasts.setState({ toasts: [] })
})

describe('a newer HQ than the open page', () => {
  test('min_client above the page level is older, equal or missing is not', () => {
    expect(pageIsOlder(CLIENT_LEVEL + 1)).toBe(true)
    expect(pageIsOlder(CLIENT_LEVEL)).toBe(false)
    expect(pageIsOlder(undefined)).toBe(false)
    expect(pageIsOlder('2')).toBe(false)
  })

  test('check 1: a hello with a newer min_client shows the V1 toast with Reload, once', () => {
    noteHelloVersion({ min_client: CLIENT_LEVEL + 1 })
    noteHelloVersion({ min_client: CLIENT_LEVEL + 1 })
    const toasts = useToasts.getState().toasts
    expect(toasts.map((x) => x.text)).toEqual([NEWER_TEXT])
    expect(toasts[0].action?.label).toBe('Reload')
  })

  test('a hello that is not newer shows nothing', () => {
    noteHelloVersion({ min_client: CLIENT_LEVEL })
    expect(useToasts.getState().toasts).toEqual([])
  })

  test('V4: a 426 from HQ shows the toast and the failure line names the action', async () => {
    const res = new Response(JSON.stringify({ error: 'upgrade', min_client: 9 }), { status: 426 })
    const err = await readJson(res).catch((e: unknown) => e)
    expect(err).toBeInstanceOf(HqHttpError)
    expect(useToasts.getState().toasts.map((x) => x.text)).toEqual([NEWER_TEXT])
    expect(failureLine(err, 'fail_archive_chat_newer', 'other')).toBe("Couldn't archive this chat. A newer HQ is running. Reload to keep going.")
    expect(failureLine(new HqHttpError('x', 500), 'fail_archive_chat_newer', 'other')).toBe('other')
  })
})

describe('Reload keeps drafts', () => {
  test('check 2: every draft is written down before the page reloads', async () => {
    const order: string[] = []
    onBeforeReload(() => order.push('composer'))
    onBeforeReload(() => order.push('note'))
    onBeforeReload(() => {
      throw new Error('storage blocked')
    })
    await reloadForNewer(() => order.push('reload'))
    expect(order).toEqual(['composer', 'note', 'reload'])
  })

  test('unsaved text survives one reload and is handed back once', () => {
    const store = new Map<string, string>()
    ;(globalThis as { sessionStorage?: unknown }).sessionStorage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    }
    keepUnsaved('note:a.md', 'half typed')
    expect(takeUnsaved('note:a.md')).toBe('half typed')
    expect(takeUnsaved('note:a.md')).toBeNull()
  })
})
