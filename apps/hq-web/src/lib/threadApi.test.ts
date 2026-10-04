import { expect, test } from 'bun:test'
import { parseToolArgs, toThreadMessage } from './threadApi'

test('a saved reply brings back its tool steps, reasoning and stop flag', () => {
  const m = toThreadMessage({
    message_id: 'a1',
    thread_id: 't',
    role: 'assistant',
    content: 'answer',
    created_at: '2026-09-26T00:00:00Z',
    meta: {
      reasoning: 'thought',
      stopped: true,
      tool_steps: [{ id: 'c1', name: 'bash', args: '{"command":"ls"}', result: 'ok', duration_ms: 12 }],
    },
  })
  expect(m.reasoning).toBe('thought')
  expect(m.stopped).toBe(true)
  expect(m.toolSteps?.[0]).toMatchObject({ toolCallId: 'c1', toolName: 'bash', status: 'done', durationMs: 12, resultOutput: 'ok' })
  expect(m.toolSteps?.[0].inputArgs).toEqual({ command: 'ls' })
})

test('a user message splits out its attachments and has no meta', () => {
  const m = toThreadMessage({
    message_id: 'u1',
    thread_id: 't',
    role: 'user',
    content: 'see\n\n<hq-attachments>[{"name":"a.png","path":"_media/web/d/a.png","mime":"image/png","size":3}]</hq-attachments>',
    created_at: '2026-09-26T00:00:00Z',
  })
  expect(m.content).toBe('see')
  expect(m.attachments?.[0].name).toBe('a.png')
  expect(m.toolSteps).toBeUndefined()
})

test('tool arguments that are not an object are still shown', () => {
  expect(parseToolArgs(undefined)).toBeUndefined()
  expect(parseToolArgs('[1,2]')).toEqual({ value: [1, 2] })
  expect(parseToolArgs('not json')).toEqual({ raw: 'not json' })
})

test('a message posted for a watched session carries its driver meta', () => {
  const base = { message_id: 'd1', thread_id: 't', role: 'assistant', content: 'done', created_at: '2026-09-26T00:00:00Z' }
  const driven = toThreadMessage({ ...base, meta: { driver: { session_id: 'hs-1', reason: 'blocked', mode: 'drive' } } })
  expect(driven.driver).toEqual({ sessionId: 'hs-1', reason: 'blocked', mode: 'drive' })
  expect(toThreadMessage({ ...base, meta: { driver: null } }).driver).toBeUndefined()
})

test('a question an MCP client asked carries its caller, and ordinary messages do not', () => {
  const base = { message_id: 'q1', thread_id: 't', role: 'user', content: 'what is open?', created_at: '2026-10-02T00:00:00Z' }
  const asked = toThreadMessage({ ...base, meta: { source: { kind: 'mcp', caller: 'claude-code', scope: 'full' } } })
  expect(asked.viaMcp).toBe('claude-code')
  expect(toThreadMessage({ ...base, meta: { source: { kind: 'mcp' } } }).viaMcp).toBe('mcp')
  expect(toThreadMessage({ ...base, meta: { source: { kind: 'other', caller: 'x' } } }).viaMcp).toBeUndefined()
  expect(toThreadMessage({ ...base, meta: null }).viaMcp).toBeUndefined()
  expect(toThreadMessage(base).viaMcp).toBeUndefined()
})
