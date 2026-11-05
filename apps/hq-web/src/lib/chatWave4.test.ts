import { expect, test } from 'bun:test'
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { bannerKey, sendFailureKey, stopFailureKey, stopReply } from './chatConnection'
import { groupChats } from './chatGroups'
import { startSuggestions } from './chatStart'
import { enterSendsMessage, type EnterInput } from './enterRule'
import { toThreadMessage, type ApiMessage } from './threadApi'
import { handleChatEvent } from './chatEvents'
import { useThreadStore } from '~/store/threadStore'

const enter = (over: Partial<EnterInput> = {}): EnterInput => ({
  key: 'Enter', shiftKey: false, isComposing: false, keyCode: 13, coarse: false, paletteActive: false, enterSends: null, ...over,
})

test('Enter sends, except while composing, with Shift, on a touch screen or in the slash palette', () => {
  expect(enterSendsMessage(enter())).toBe(true)
  expect(enterSendsMessage(enter({ isComposing: true }))).toBe(false)
  expect(enterSendsMessage(enter({ keyCode: 229 }))).toBe(false)
  expect(enterSendsMessage(enter({ shiftKey: true }))).toBe(false)
  expect(enterSendsMessage(enter({ coarse: true }))).toBe(false)
  expect(enterSendsMessage(enter({ paletteActive: true }))).toBe(false)
  expect(enterSendsMessage(enter({ enterSends: false }))).toBe(false)
})

test('Stop goes over HTTP when the socket is down, and fails only when neither reaches HQ', async () => {
  const calls: string[] = []
  expect(await stopReply('t1', () => false, async (id) => void calls.push(id))).toBe(true)
  expect(calls).toEqual(['t1'])
  expect(await stopReply('t1', () => true, async (id) => void calls.push(id))).toBe(true)
  expect(calls).toEqual(['t1'])
  expect(await stopReply('t1', () => false, async () => { throw new Error('down') })).toBe(false)
})

test('each connection state has its own banner and failure line', () => {
  expect(bannerKey('connected', false)).toBeNull()
  expect(bannerKey('starting', false)).toBe('chat_banner_starting')
  expect(bannerKey('unreachable', true)).toBe('chat_banner_shared')
  expect(stopFailureKey('offline')).toBe('fail_stop_reply_offline')
  expect(sendFailureKey('unreachable')).toBe('fail_send_message_unreachable')
})

test('chats group by the day they last moved', () => {
  const now = new Date(2026, 4, 20, 15).getTime()
  const day = 86_400_000
  const groups = groupChats([{ updatedAt: now - 1000 }, { updatedAt: now - day }, { updatedAt: now - 4 * day }, { updatedAt: now - 30 * day }], now)
  expect(groups.map((g) => g.key)).toEqual(['today', 'yesterday', 'week', 'older'])
  expect(groupChats([{ updatedAt: now }], now).map((g) => g.key)).toEqual(['today'])
})

test('suggestions fill the box: the note one only with a note attached', () => {
  expect(startSuggestions(false)).toHaveLength(3)
  expect(startSuggestions(true)[0]).toContain('note')
  expect(startSuggestions(false)[0]).toContain('week')
})

test('a message asked by voice is marked spoken', () => {
  const m: ApiMessage = { message_id: 'm', thread_id: 't', role: 'user', content: 'hi', created_at: new Date().toISOString(), meta: { source: 'voice' } }
  expect(toThreadMessage(m).spoken).toBe(true)
  expect(toThreadMessage({ ...m, meta: null }).spoken).toBeUndefined()
})

test('a resumed reply shows what it wrote so far, and turn_end records that HQ waits for the owner', () => {
  const tid = 'resume-t'
  handleChatEvent({ type: 'reply_so_far', thread_id: tid, running: true, content: 'Partly' })
  expect(useThreadStore.getState().live[tid]?.content).toBe('Partly')
  handleChatEvent({ type: 'turn_end', thread_id: tid, message_id: 'm1', waiting_for_owner: true })
  expect(useThreadStore.getState().waiting[tid]).toBe(true)
})

test('the chat screens use none of the retired words', () => {
  const dir = join(import.meta.dir, '..', 'components', 'chat')
  const retired = ['Not connected, so', 'Picks up where it was', "HQ did not finish a reply", 'Connection lost', 'not rendered:']
  for (const f of readdirSync(dir).filter((n) => n.endsWith('.tsx'))) {
    const src = readFileSync(join(dir, f), 'utf8')
    for (const w of retired) expect(src.includes(w)).toBe(false)
  }
})
