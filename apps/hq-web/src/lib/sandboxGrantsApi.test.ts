import { afterEach, describe, expect, test } from 'bun:test'
import {
  activeResourceGrants,
  bindSandboxProfile,
  historyResourceGrants,
  isSandboxGrantNotification,
  listSandboxGrants,
  listSandboxProfiles,
  parseSandboxGrantNotification,
  pendingResourceGrants,
  revokeSandboxGrant,
  taskProfileFromGrants,
  type SandboxGrant,
} from './sandboxGrantsApi'
import { approvalsForThread } from './chatApprovals'
import type { NotificationItem } from './notificationsApi'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

const makeGrant = (over: Partial<SandboxGrant> = {}): SandboxGrant => ({
  id: 'g-1',
  session_id: 'hs-1',
  task_id: 'TASK-42',
  host: 'local',
  repo: '/home/user/repo',
  capability: 'fs_write',
  resource: '/home/user/.gradle',
  state: 'pending',
  decided_by: 'system',
  helper_reason: 'Need gradle cache for build',
  decision_note: '',
  requested_at: 1_700_000_000,
  decided_at: null,
  applied_at: null,
  expires_at: 1_700_086_400,
  revoked_at: null,
  ...over,
})

const makeNotif = (over: Partial<NotificationItem> = {}): NotificationItem => ({
  id: 'val_1',
  kind: 'action_needed',
  category: 'value',
  title: 'A helper wants to write to /home/user/.gradle',
  description:
    'Helper session hs-1 asked to write to /home/user/.gradle. HQ could not approve it by itself: it is outside the access this task was given. Approving applies to this session only and lasts up to a day; the session restarts to pick it up. The helper\'s own words, not HQ\'s: "Need gradle cache for build". Dismiss to refuse.',
  state: 'pending',
  created_at: '2026-11-20T08:00:00Z',
  metadata: {
    extra: {
      source_task: 'sandbox-grants',
      val_kind: 'action_needed',
    },
  },
  actions: ['approve', 'reject'],
  ...over,
})

describe('sandboxGrantsApi REST client', () => {
  test('listSandboxGrants calls GET /api/sandbox-grants with optional session query', async () => {
    const calls: { url: string; method?: string }[] = []
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ url: String(input), method: init?.method })
      return new Response(JSON.stringify({ grants: [makeGrant()] }), { status: 200 })
    }) as typeof fetch

    const all = await listSandboxGrants()
    expect(all.grants).toHaveLength(1)
    const one = await listSandboxGrants('hs-1/a')
    expect(one.grants).toHaveLength(1)

    expect(calls[0].url).toEndWith('/api/sandbox-grants')
    expect(calls[1].url).toEndWith('/api/sandbox-grants?session=hs-1%2Fa')
  })

  test('revokeSandboxGrant calls POST /api/sandbox-grants/{id}/revoke', async () => {
    let seenUrl = ''
    let seenMethod = ''
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      seenUrl = String(input)
      seenMethod = init?.method ?? ''
      return new Response(JSON.stringify({ revoked: 'g-9' }), { status: 200 })
    }) as typeof fetch

    const res = await revokeSandboxGrant('g-9')
    expect(res).toEqual({ revoked: 'g-9' })
    expect(seenMethod).toBe('POST')
    expect(seenUrl).toEndWith('/api/sandbox-grants/g-9/revoke')
  })

  test('listSandboxProfiles calls GET /api/sandbox-grants/profiles', async () => {
    let seenUrl = ''
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      seenUrl = String(input)
      return new Response(
        JSON.stringify({
          profiles: [
            {
              name: 'node-build',
              writable: ['/home/user/.npm'],
              readable: ['/home/user/sdk'],
              loopback_ports: [5173],
              egress: ['registry.example.org'],
            },
          ],
        }),
        { status: 200 },
      )
    }) as typeof fetch

    const res = await listSandboxProfiles()
    expect(seenUrl).toEndWith('/api/sandbox-grants/profiles')
    expect(res.profiles[0].name).toBe('node-build')
    expect(res.profiles[0].egress).toEqual(['registry.example.org'])
  })

  test('bindSandboxProfile calls POST /api/sandbox-grants/profile with scope, id and name', async () => {
    let seenUrl = ''
    let seenMethod = ''
    let seenBody = ''
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      seenUrl = String(input)
      seenMethod = init?.method ?? ''
      seenBody = String(init?.body ?? '')
      return new Response(JSON.stringify({ bound: 'rust-build' }), { status: 200 })
    }) as typeof fetch

    const res = await bindSandboxProfile({ scope: 'task', id: 'TASK-42', name: 'rust-build' })
    expect(res).toEqual({ bound: 'rust-build' })
    expect(seenMethod).toBe('POST')
    expect(seenUrl).toEndWith('/api/sandbox-grants/profile')
    expect(JSON.parse(seenBody)).toEqual({ scope: 'task', id: 'TASK-42', name: 'rust-build' })
  })
})

describe('sandbox-grants helpers and notification parsing', () => {
  test('filters active, pending, history and ignores profile binding rows', () => {
    const grants: SandboxGrant[] = [
      makeGrant({ id: 'p', capability: 'profile', session_id: 'task:TASK-42', resource: 'node-build', state: 'active' }),
      makeGrant({ id: 'a', state: 'active', decided_by: 'policy' }),
      makeGrant({ id: 'w', state: 'pending' }),
      makeGrant({ id: 'r', state: 'revoked', decision_note: 'Stopped by owner' }),
      makeGrant({ id: 'e', state: 'expired' }),
      makeGrant({ id: 'x', state: 'refused', decision_note: 'Credential directory' }),
    ]

    expect(activeResourceGrants(grants).map((g) => g.id)).toEqual(['a'])
    expect(pendingResourceGrants(grants).map((g) => g.id)).toEqual(['w'])
    expect(historyResourceGrants(grants).map((g) => g.id)).toEqual(['r', 'e', 'x'])
    expect(taskProfileFromGrants(grants, 'TASK-42')).toBe('node-build')
    expect(taskProfileFromGrants(grants, 'TASK-99')).toBe('none')
  })

  test('parses sandbox-grants notification and enriches with matching grant', () => {
    const n = makeNotif()
    expect(isSandboxGrantNotification(n)).toBe(true)

    const parsedOnly = parseSandboxGrantNotification(n)
    expect(parsedOnly.sessionId).toBe('hs-1')
    expect(parsedOnly.capability).toBe('fs_write')
    expect(parsedOnly.resource).toBe('/home/user/.gradle')
    expect(parsedOnly.helperReason).toBe('Need gradle cache for build')
    expect(parsedOnly.taskId).toBeNull()

    const enriched = parseSandboxGrantNotification(n, [makeGrant()])
    expect(enriched.taskId).toBe('TASK-42')
  })

  test('approvalsForThread includes sandbox-grants notifications for watched sessions', () => {
    const n1 = makeNotif({ id: 'val_1' })
    const n2 = makeNotif({
      id: 'val_2',
      description: 'Helper session hs-other asked to reach api.example.org:443. HQ could not approve it.',
    })
    const matched = approvalsForThread([n1, n2], 'thread-1', ['hs-1'])
    expect(matched.map((i) => i.id)).toEqual(['val_1'])
  })
})
