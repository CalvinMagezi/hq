import { expect, test } from 'bun:test'
import type { ChatAppStatus, ChatAppsStatus } from './chatAppsApi'
import { HqHttpError } from './hqAuth'
import {
  DEFAULT_GET_STARTED_PREFS,
  chatAppConnected,
  getStartedItems,
  getStartedVisible,
  isGetStartedPrefs,
  serverUnreachable,
  type GetStartedSignals,
} from './getStarted'

const app = (token_set: boolean, paired: boolean): ChatAppStatus =>
  ({ token_set, owner: { paired, id: null, from_config: false } }) as ChatAppStatus

const apps = (telegram: ChatAppStatus, discord: ChatAppStatus): ChatAppsStatus =>
  ({ telegram, discord }) as ChatAppsStatus

const signals = (over: Partial<GetStartedSignals> = {}): GetStartedSignals => ({
  prefs: DEFAULT_GET_STARTED_PREFS,
  standalone: false,
  chatApps: undefined,
  push: 'off',
  ...over,
})

const doneById = (s: GetStartedSignals) => Object.fromEntries(getStartedItems(s).map((i) => [i.id, i.done]))

test('a chat app counts only with a saved token and a paired owner', () => {
  expect(chatAppConnected(undefined)).toBe(false)
  expect(chatAppConnected(apps(app(true, false), app(false, false)))).toBe(false)
  expect(chatAppConnected(apps(app(false, true), app(false, false)))).toBe(false)
  expect(chatAppConnected(apps(app(true, true), app(false, false)))).toBe(true)
  expect(chatAppConnected(apps(app(false, false), app(true, true)))).toBe(true)
})

test('items start undone and link to their settings sections', () => {
  const items = getStartedItems(signals())
  expect(items.map((i) => [i.id, i.to, i.hash, i.done])).toEqual([
    ['phone', '/settings', 'devices', false],
    ['chat', '/settings', 'chat-apps', false],
    ['notifications', '/notifications', undefined, false],
  ])
})

test('the phone step is done when installed or marked done', () => {
  expect(doneById(signals({ standalone: true })).phone).toBe(true)
  expect(doneById(signals({ prefs: { hidden: false, phoneDone: true } })).phone).toBe(true)
})

test('notifications are done only when this device is subscribed, and dropped when they cannot be turned on', () => {
  expect(doneById(signals({ push: 'on' })).notifications).toBe(true)
  expect(doneById(signals({ push: null })).notifications).toBe(false)
  expect(doneById(signals({ push: 'needs-install' })).notifications).toBe(false)
  expect(getStartedItems(signals({ push: 'unsupported' })).map((i) => i.id)).toEqual(['phone', 'chat'])
  expect(getStartedItems(signals({ push: 'server-off' })).map((i) => i.id)).toEqual(['phone', 'chat'])
})

test('visible only once settled, signed in, not hidden and with something left to do', () => {
  const open = getStartedItems(signals())
  const finished = getStartedItems(
    signals({ standalone: true, chatApps: apps(app(true, true), app(false, false)), push: 'on' }),
  )
  const base = { settled: true, unavailable: false, prefs: DEFAULT_GET_STARTED_PREFS, items: open }
  expect(getStartedVisible(base)).toBe(true)
  expect(getStartedVisible({ ...base, settled: false })).toBe(false)
  expect(getStartedVisible({ ...base, unavailable: true })).toBe(false)
  expect(getStartedVisible({ ...base, prefs: { hidden: true, phoneDone: false } })).toBe(false)
  expect(getStartedVisible({ ...base, items: finished })).toBe(false)
})

test('stored prefs of the wrong shape are rejected', () => {
  expect(isGetStartedPrefs({ hidden: true, phoneDone: false })).toBe(true)
  expect(isGetStartedPrefs({ hidden: 'yes', phoneDone: false })).toBe(false)
  expect(isGetStartedPrefs(null)).toBe(false)
  expect(isGetStartedPrefs(true)).toBe(false)
})

test('network failures and gateway errors count as unreachable, other HTTP errors do not', () => {
  expect(serverUnreachable(new TypeError('Failed to fetch'))).toBe(true)
  expect(serverUnreachable(new HqHttpError('Bad Gateway', 502))).toBe(true)
  expect(serverUnreachable(new HqHttpError('not found', 404))).toBe(false)
  expect(serverUnreachable(new HqHttpError('boom', 500))).toBe(false)
})
