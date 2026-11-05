import { expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { approvalView, approvalsForThread, splitSaid } from './chatApprovals'
import { autoAnswers } from './autoAnswers'
import { isSpokenThread } from './chatGroups'
import { handleChatEvent } from './chatEvents'
import { ApprovalCard } from '~/components/ui/ApprovalCard'
import { SessionsPanel } from '~/components/chat/SessionsPanel'
import { useThreadStore } from '~/store/threadStore'
import { t } from '~/strings'
import type { NotificationItem } from './notificationsApi'
import type { WatchedSession } from './sessionsApi'

const item = (id: string, thread: string | null, over: Partial<NotificationItem> = {}): NotificationItem => ({
  id, kind: 'action_needed', category: 'approval', title: 'HQ needs your OK', description: 'Delete the task.', state: 'pending',
  created_at: '2026-11-05T11:00:00Z', actions: ['approve', 'reject'],
  metadata: { approval: { tool: 'task_delete', args: { id: 'T-9' }, loses: true, verb: 'delete', thread_id: thread } }, ...over,
})

test('a chat shows only the approvals that name it, never another chat app', () => {
  const list = [item('a', 't1'), item('b', 't2'), item('c', null)]
  expect(approvalsForThread(list, 't1').map((i) => i.id)).toEqual(['a'])
  expect(approvalsForThread(list, null)).toEqual([])
})

test('an item answered elsewhere reads as already answered, one answered here does not', () => {
  const done = item('a', 't1', { state: 'approved' })
  expect(approvalView(done, new Set())).toBe('answered_elsewhere')
  expect(approvalView(done, new Set(['a']))).toBe('answered_here')
  expect(approvalView(item('b', 't1'), new Set())).toBe('pending')
  expect(t('chat_approval_elsewhere')).toBe('Already answered. Someone answered this somewhere else.')
})

test('a voice turn quotes what was said above HQ\'s own words', () => {
  expect(splitSaid('You said: “Check the repo.”\nShow changed files.')).toEqual({ said: 'You said: “Check the repo.”', rest: 'Show changed files.' })
  expect(splitSaid('Delete the task.')).toEqual({ said: null, rest: 'Delete the task.' })
})

test('the card of a losing request quotes the tool and arguments with a red Allow delete', () => {
  const html = renderToStaticMarkup(
    createElement(ApprovalCard, { tool: 'task_delete', args: { id: 'T-9' }, quoteTitle: t('approval_quote_chat'), loses: true, verb: 'delete', onDecline() {}, onAllow() {} }),
  )
  expect(html).toContain('task_delete')
  expect(html).toContain('{&quot;id&quot;: &quot;T-9&quot;}')
  expect(html).toContain('a-btn danger')
  expect(html).toContain('Allow delete')
})

test('turn_end with waiting_for_owner puts the chat in the needs-your-OK state', () => {
  handleChatEvent({ type: 'turn_end', thread_id: 'w1', waiting_for_owner: true } as never)
  expect(useThreadStore.getState().waiting.w1).toBe(true)
  handleChatEvent({ type: 'turn_end', thread_id: 'w1' } as never)
  expect(useThreadStore.getState().waiting.w1).toBe(false)
})

test('answers HQ gave are read from the answered events, the owner own answers left out', () => {
  const events = [
    { kind: 'answered', by: 'a Drive turn', at: '2026-11-05 11:12:00', detail: JSON.stringify({ by: 'a Drive turn', kind: 'question', choice: { option: 1 } }) },
    { kind: 'answered', by: 'owner', at: '2026-11-05 11:13:00', detail: JSON.stringify({ by: 'owner', kind: 'edit', choice: { option: 1 } }) },
    { kind: 'answered', by: 'the command policy', at: '2026-11-05 11:14:00', detail: JSON.stringify({ by: 'the command policy', kind: 'command', choice: 'decline' }) },
    { kind: 'stopped', by: 'x' },
    { kind: 'answered', by: 'broken', detail: '{' },
  ]
  const out = autoAnswers(events)
  expect(out.map((a) => a.by)).toEqual(['a Drive turn', 'the command policy'])
  expect(out[1].declined).toBe(true)
  expect(autoAnswers(undefined)).toEqual([])
})

const session = { id: 'hs-1', harness: 'claude', label: 'Gradle', host: 'native', cwd: '/x', status: 'running', agent_status: 'idle', last_seen_at: null, drive: false, mode: 'observe', goal: null, done_criteria: null, drive_blocked_by: [], drive_off_reason: null, nudges_sent: 0, pending_wake: null, last_driven_at: null, created_at: '2026-11-05 10:00:00', task: null } as WatchedSession

test('the details panel says what Let HQ say yes does, lists answers or an empty state, and notes alerts', () => {
  const html = renderToStaticMarkup(createElement(SessionsPanel, { sessions: [session], busy: new Set(), onDrive() {}, onUnwatch() {} }))
  expect(html).toContain(t('chat_say_yes'))
  expect(html).toContain(t('chat_say_yes_off').replace("'", '&#x27;'))
  expect(html).toContain(t('chat_say_yes_none'))
  expect(html).toContain(t('chat_alerts_note'))
  expect(html).not.toContain('Drive')
})

test('a chat started by voice carries the spoken mark in the list', () => {
  expect(isSpokenThread([{ role: 'user', spoken: true }, { role: 'assistant' }])).toBe(true)
  expect(isSpokenThread([{ role: 'user' }])).toBe(false)
  expect(isSpokenThread(undefined)).toBe(false)
})
