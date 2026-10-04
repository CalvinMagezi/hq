import { beforeEach, expect, test } from 'bun:test'
import { useThreadStore } from '~/store/threadStore'
import { flushPendingDeltas, handleChatEvent } from './chatEvents'

const live = (tid: string) => useThreadStore.getState().live[tid]

beforeEach(() => {
  flushPendingDeltas()
  useThreadStore.setState({ threads: [], threadMessages: {}, live: {}, activeThreadId: null })
  // No server here: any reload the handlers start simply fails, which they must survive.
  globalThis.fetch = (() => Promise.reject(new Error('offline'))) as unknown as typeof fetch
})

test('a long stream is written to the store in a few batches, in order', () => {
  let writes = 0
  const unsubscribe = useThreadStore.subscribe(() => writes++)
  handleChatEvent({ type: 'turn_start', thread_id: 't' })
  const before = writes
  for (let i = 0; i < 5000; i++) handleChatEvent({ type: 'text_delta', thread_id: 't', content: `${i},` })
  flushPendingDeltas()
  unsubscribe()
  expect(live('t').content.startsWith('0,1,2,')).toBe(true)
  expect(live('t').content.endsWith('4999,')).toBe(true)
  expect(writes - before).toBeLessThan(5)
})

test('an event that is not a delta sees every delta before it', () => {
  handleChatEvent({ type: 'turn_start', thread_id: 't' })
  handleChatEvent({ type: 'text_delta', thread_id: 't', content: 'partial answer' })
  handleChatEvent({ type: 'turn_end', thread_id: 't', message_id: 'm1', stopped: true })
  const saved = useThreadStore.getState().threadMessages.t
  expect(saved.at(-1)?.content).toBe('partial answer')
  expect(saved.at(-1)?.stopped).toBe(true)
  expect(live('t')).toBeUndefined()
})

test('a saved user message that cannot be read still leaves the reply running', () => {
  expect(() => handleChatEvent({ type: 'turn_start', thread_id: 't', user_message: 42 })).not.toThrow()
  expect(live('t')).toBeDefined()
})

test('malformed event fields never throw or blank the chat', () => {
  handleChatEvent({ type: 'turn_start', thread_id: 't' })
  expect(() => {
    handleChatEvent({ type: 'text_delta', thread_id: 't', content: { nested: true } })
    handleChatEvent({ type: 'tool_start', thread_id: 't', tool_call_id: 7, tool_name: null, args: '{broken' })
    handleChatEvent({ type: 'tool_end', thread_id: 't', result: 12 })
    handleChatEvent({ type: 'error', thread_id: 't' })
    handleChatEvent({ type: 'thread_title', thread_id: 't', title: 5 })
  }).not.toThrow()
  flushPendingDeltas()
  expect(live('t').toolSteps.length).toBe(1)
  expect(live('t').content).toContain('Error')
})

test('a repeated tool start and endless progress do not grow the tool list without bound', () => {
  handleChatEvent({ type: 'turn_start', thread_id: 't' })
  handleChatEvent({ type: 'tool_start', thread_id: 't', tool_call_id: 'c1', tool_name: 'bash' })
  handleChatEvent({ type: 'tool_start', thread_id: 't', tool_call_id: 'c1', tool_name: 'bash' })
  for (let i = 0; i < 1000; i++) handleChatEvent({ type: 'tool_progress', thread_id: 't', tool_call_id: 'c1', message: `${i}` })
  const [step, ...rest] = live('t').toolSteps
  expect(rest).toHaveLength(0)
  expect(step.progressMessages.length).toBeLessThanOrEqual(20)
  expect(step.progressMessages.at(-1)).toBe('999')
})

test('tool calls with no id each get their own row', () => {
  handleChatEvent({ type: 'turn_start', thread_id: 't' })
  handleChatEvent({ type: 'tool_start', thread_id: 't', tool_name: 'a' })
  handleChatEvent({ type: 'tool_start', thread_id: 't', tool_name: 'b' })
  expect(new Set(live('t').toolSteps.map((s) => s.toolCallId)).size).toBe(2)
})

test('two chats streaming interleaved stay separate through the batching', () => {
  handleChatEvent({ type: 'turn_start', thread_id: 'a' })
  handleChatEvent({ type: 'turn_start', thread_id: 'b' })
  handleChatEvent({ type: 'text_delta', thread_id: 'a', content: 'A1 ' })
  handleChatEvent({ type: 'text_delta', thread_id: 'b', content: 'B1 ' })
  handleChatEvent({ type: 'text_delta', thread_id: 'a', content: 'A2' })
  handleChatEvent({ type: 'turn_end', thread_id: 'b', message_id: 'mb' })
  flushPendingDeltas()
  expect(live('a').content).toBe('A1 A2')
  expect(live('b')).toBeUndefined()
  expect(useThreadStore.getState().threadMessages.b.at(-1)?.content).toBe('B1 ')
})

test('after the server says the socket lagged, running replies are flagged as incomplete', () => {
  handleChatEvent({ type: 'turn_start', thread_id: 't' })
  handleChatEvent({ type: 'text_delta', thread_id: 't', content: 'so far' })
  expect(handleChatEvent({ type: 'stream_lag', skipped: 12 })).toBe(true)
  expect(live('t').gap).toBe(true)
  expect(live('t').content).toBe('so far')
})

test('step credits attach to the first tool call of their step and stay with the saved reply', () => {
  handleChatEvent({ type: 'turn_start', thread_id: 't' })
  handleChatEvent({ type: 'tool_start', thread_id: 't', tool_call_id: 'a', tool_name: 'bash' })
  handleChatEvent({ type: 'tool_start', thread_id: 't', tool_call_id: 'b', tool_name: 'bash' })
  handleChatEvent({ type: 'step_credits', thread_id: 't', turn: 1, delta: 4 })
  handleChatEvent({ type: 'step_credits', thread_id: 't', turn: 2, delta: null })
  expect(live('t').stepCredits.map((c) => [c.toolCallId, c.delta])).toEqual([['a', 4], [undefined, null]])
  handleChatEvent({ type: 'turn_end', thread_id: 't', message_id: 'm1' })
  expect(useThreadStore.getState().threadMessages.t.at(-1)?.stepCredits?.length).toBe(2)
})

test('a rejected send removes the local copy and clears the busy turn', () => {
  useThreadStore.getState().appendMessage('t', { messageId: 'local-1', threadId: 't', role: 'user', content: 'hi', createdAt: 1 })
  useThreadStore.getState().startTurn('t')
  handleChatEvent({ type: 'chat_rejected', thread_id: 't', client_id: 'local-1', reason: 'busy', running: false })
  expect(live('t')).toBeUndefined()
  expect(useThreadStore.getState().threadMessages.t).toHaveLength(0)
})
