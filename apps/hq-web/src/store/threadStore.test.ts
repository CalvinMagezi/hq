import { expect, test } from 'bun:test'
import { useThreadStore } from './threadStore'

const thread = (threadId: string) => ({
  threadId, title: threadId, status: 'active' as const, createdAt: 0, updatedAt: 0, unreadCount: 0,
})

test('two chats stream at once without mixing, and a background finish counts as unread', () => {
  const s = useThreadStore.getState()
  s.setThreads([thread('a'), thread('b')])
  s.setActiveThread('a')
  s.startTurn('a')
  s.startTurn('b')
  s.appendContent('a', 'alpha ')
  s.appendContent('b', 'beta ')
  s.appendContent('a', 'one')
  s.startToolStep('b', 'call-1', 'bash')
  s.endToolStep('b', 'call-1', 'ok')

  let live = useThreadStore.getState().live
  expect(live.a.content).toBe('alpha one')
  expect(live.b.content).toBe('beta ')
  expect(live.b.toolSteps[0].status).toBe('done')

  s.finalizeTurn('b', { seen: true })
  s.finalizeTurn('a', { seen: true })
  const st = useThreadStore.getState()
  expect(st.live).toEqual({})
  expect(st.threadMessages.a.at(-1)?.content).toBe('alpha one')
  expect(st.threadMessages.b.at(-1)?.toolSteps?.length).toBe(1)
  expect(st.threads.find((t) => t.threadId === 'b')?.unreadCount).toBe(1)
  expect(st.threads.find((t) => t.threadId === 'a')?.unreadCount).toBe(0)

  useThreadStore.getState().setActiveThread('b')
  expect(useThreadStore.getState().threads.find((t) => t.threadId === 'b')?.unreadCount).toBe(0)
})

test('syncRunning keeps server-running chats and reports ones that finished offline', () => {
  const s = useThreadStore.getState()
  s.startTurn('x')
  s.appendContent('x', 'partial')
  s.startTurn('y')
  const finished = s.syncRunning(['x', 'z'])
  const live = useThreadStore.getState().live
  expect(finished).toEqual(['y'])
  expect(live.x.content).toBe('partial')
  expect(Object.keys(live).sort()).toEqual(['x', 'z'])
})

const msg = (id: string, role: 'user' | 'assistant' = 'user') => ({
  messageId: id, threadId: 'p', role, content: id, createdAt: 0,
})

test('a refreshed newest page keeps older pages it joins onto, and replaces a list it does not', () => {
  const s = useThreadStore.getState()
  s.setThreadMessages('p', [msg('m1'), msg('m2'), msg('m3')])
  s.mergeLatest('p', [msg('m2'), msg('m3'), msg('m4')], true)
  expect(useThreadStore.getState().threadMessages.p.map((m) => m.messageId)).toEqual(['m1', 'm2', 'm3', 'm4'])

  // A short page is everything the server has: m1 was deleted (an edit on another device).
  s.mergeLatest('p', [msg('m2'), msg('m3'), msg('m4')], false)
  expect(useThreadStore.getState().threadMessages.p.map((m) => m.messageId)).toEqual(['m2', 'm3', 'm4'])
  s.setThreadMessages('p', [msg('m1'), msg('m2'), msg('m3')])

  s.mergeLatest('p', [msg('m9')], false)
  const st = useThreadStore.getState()
  expect(st.threadMessages.p.map((m) => m.messageId)).toEqual(['m9'])
  expect(st.olderAvailable.p).toBe(false)
})

test('an older page goes in front without duplicates, and remove-from drops a message and what follows', () => {
  const s = useThreadStore.getState()
  s.setThreadMessages('p', [msg('m3'), msg('m4'), msg('m5')])
  s.prependMessages('p', [msg('m1'), msg('m2'), msg('m3')], true)
  expect(useThreadStore.getState().threadMessages.p.map((m) => m.messageId)).toEqual(['m1', 'm2', 'm3', 'm4', 'm5'])
  expect(useThreadStore.getState().olderAvailable.p).toBe(true)

  s.removeFrom('p', 'm4')
  expect(useThreadStore.getState().threadMessages.p.map((m) => m.messageId)).toEqual(['m1', 'm2', 'm3'])
})

test('the saved user message replaces its local copy here and appears on other devices', () => {
  const s = useThreadStore.getState()
  s.setThreadMessages('p', [msg('local-1')])
  s.confirmUserMessage('p', msg('srv-1'), 'local-1')
  s.confirmUserMessage('p', msg('srv-1'), 'local-1')
  expect(useThreadStore.getState().threadMessages.p.map((m) => m.messageId)).toEqual(['srv-1'])

  s.confirmUserMessage('p', msg('srv-2'))
  expect(useThreadStore.getState().threadMessages.p.map((m) => m.messageId)).toEqual(['srv-1', 'srv-2'])
})

test('a finished reply takes the server id, and one nobody saw counts as unread even in the open chat', () => {
  const s = useThreadStore.getState()
  s.setThreads([thread('p')])
  s.setActiveThread('p')
  s.setThreadMessages('p', [])
  s.startTurn('p')
  s.appendContent('p', 'done')
  expect(s.finalizeTurn('p', { messageId: 'srv-9', seen: false, stopped: true })).toBe('done')
  const st = useThreadStore.getState()
  expect(st.threadMessages.p.at(-1)).toMatchObject({ messageId: 'srv-9', stopped: true })
  expect(st.threads[0].unreadCount).toBe(1)

  s.startTurn('p')
  expect(s.finalizeTurn('p', { seen: true })).toBeNull()
})

test('the device cache never overwrites messages the server already delivered', () => {
  const merge = useThreadStore.persist.getOptions().merge!
  const current = { ...useThreadStore.getState(), threadMessages: { p: [msg('fresh')] }, activeThreadId: 'p' }
  const persisted = { threadMessages: { p: [msg('stale')], q: [msg('cached')] }, activeThreadId: 'q', threads: [] }
  const merged = merge(persisted, current) as ReturnType<typeof useThreadStore.getState>
  expect(merged.threadMessages.p.map((m) => m.messageId)).toEqual(['fresh'])
  expect(merged.threadMessages.q.map((m) => m.messageId)).toEqual(['cached'])
  expect(merged.activeThreadId).toBe('p')
})

test('threads are sorted by latest activity descending with stable tie breaking', () => {
  const s = useThreadStore.getState()
  // Given threads with different updatedAt and some ties
  const tOld = { ...thread('old'), updatedAt: 1000, createdAt: 1000 }
  const tMidA = { ...thread('mid-a'), updatedAt: 2000, createdAt: 1500 }
  const tMidZ = { ...thread('mid-z'), updatedAt: 2000, createdAt: 1500 }
  const tNew = { ...thread('new'), updatedAt: 3000, createdAt: 3000 }

  s.setThreads([tOld, tMidA, tNew, tMidZ])
  const ids = useThreadStore.getState().threads.map((t) => t.threadId)
  // Expected order: tNew (3000), then tMidZ (2000, z > a), tMidA (2000), then tOld (1000)
  expect(ids).toEqual(['new', 'mid-z', 'mid-a', 'old'])
})

test('meaningful activity (appendMessage, finalizeTurn, confirmUserMessage) reorders threads to top', () => {
  const s = useThreadStore.getState()
  const t1 = { ...thread('t1'), updatedAt: 1000, createdAt: 1000 }
  const t2 = { ...thread('t2'), updatedAt: 2000, createdAt: 2000 }
  s.setThreads([t1, t2])
  expect(useThreadStore.getState().threads.map((t) => t.threadId)).toEqual(['t2', 't1'])

  // User sends a message on t1: t1 becomes most recent and jumps to index 0
  s.appendMessage('t1', {
    messageId: 'm-user',
    threadId: 't1',
    role: 'user',
    content: 'hello from user',
    createdAt: 3000,
  })
  expect(useThreadStore.getState().threads.map((t) => t.threadId)).toEqual(['t1', 't2'])
  expect(useThreadStore.getState().threads[0].lastMessagePreview).toBe('hello from user')

  // Assistant turn finalizes on t2 with newer content: t2 jumps back to top
  s.startTurn('t2')
  s.appendContent('t2', 'assistant response')
  s.finalizeTurn('t2', { seen: true })
  expect(useThreadStore.getState().threads.map((t) => t.threadId)).toEqual(['t2', 't1'])
  expect(useThreadStore.getState().threads[0].lastMessagePreview).toBe('assistant response')
})
