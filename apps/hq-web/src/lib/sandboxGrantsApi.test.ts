import { afterEach, describe, expect, test } from 'bun:test'
import {
  activeResourceGrants,
  bindSandboxProfile,
  formatSandboxSummary,
  formatStandingRule,
  getSandboxSummary,
  historyResourceGrants,
  isSandboxGrantNotification,
  isStandingRule,
  listSandboxGrants,
  listSandboxProfiles,
  listSandboxRules,
  listSandboxStarters,
  parseSandboxGrantNotification,
  pendingResourceGrants,
  revokeSandboxGrant,
  taskProfileFromGrants,
  type SandboxGrant,
} from './sandboxGrantsApi'
import { approvalsForThread } from './chatApprovals'
import { executeNotificationAction, type NotificationItem } from './notificationsApi'

const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

const makeGrant = (over: Partial<SandboxGrant> = {}): SandboxGrant => ({
  id: 'g-1',
  session_id: 'hs-1',
  task_id: 'TASK-42',
  host: 'local',
  repo: '/work/app',
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
  scope: 'session',
  suggested_starter: 'mobile-dev',
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
    expect(all.grants[0].suggested_starter).toBe('mobile-dev')
    const one = await listSandboxGrants('hs-1/a')
    expect(one.grants).toHaveLength(1)

    expect(calls[0].url).toEndWith('/api/sandbox-grants')
    expect(calls[1].url).toEndWith('/api/sandbox-grants?session=hs-1%2Fa')
  })

  test('listSandboxRules, listSandboxStarters and getSandboxSummary call the P6 endpoints', async () => {
    const urls: string[] = []
    globalThis.fetch = (async (input: RequestInfo | URL) => {
      const url = String(input)
      urls.push(url)
      if (url.endsWith('/rules')) {
        return new Response(
          JSON.stringify({
            rules: [makeGrant({ id: 'r-1', scope: 'repo', session_id: 'repo:/work/app', resource: '~/.gradle', state: 'active' })],
          }),
          { status: 200 },
        )
      }
      if (url.endsWith('/starters')) {
        return new Response(
          JSON.stringify({
            starters: [
              { name: 'mobile-dev', writable: ['~/.gradle'], readable: [], loopback_ports: [5037], egress: ['packages.example.org'] },
            ],
          }),
          { status: 200 },
        )
      }
      return new Response(
        JSON.stringify({
          days: 7,
          summary: {
            approved_by_policy: 12,
            approved_by_owner: 2,
            asked_owner: 3,
            declined: 1,
            refused: 1,
            standing_rules: 4,
          },
        }),
        { status: 200 },
      )
    }) as typeof fetch

    const rules = await listSandboxRules()
    const starters = await listSandboxStarters()
    const summary = await getSandboxSummary(7)

    expect(urls[0]).toEndWith('/api/sandbox-grants/rules')
    expect(urls[1]).toEndWith('/api/sandbox-grants/starters')
    expect(urls[2]).toEndWith('/api/sandbox-grants/summary?days=7')
    expect(rules.rules[0].scope).toBe('repo')
    expect(starters.starters[0].name).toBe('mobile-dev')
    expect(summary.summary.approved_by_policy).toBe(12)
  })

  test('executeNotificationAction forwards starter when answering allow_starter', async () => {
    let seenUrl = ''
    let seenBody = ''
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      seenUrl = String(input)
      seenBody = String(init?.body ?? '')
      return new Response(JSON.stringify({ success: true, new_state: 'approved', message: 'ok' }), { status: 200 })
    }) as typeof fetch

    await executeNotificationAction('val_1', 'allow_starter', { starter: 'mobile-dev' })
    expect(seenUrl).toEndWith('/api/notifications/val_1/action')
    expect(JSON.parse(seenBody)).toEqual({ action: 'allow_starter', starter: 'mobile-dev' })
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
  test('filters active, pending, history and separates standing rules and profile bindings', () => {
    const grants: SandboxGrant[] = [
      makeGrant({ id: 'p', capability: 'profile', session_id: 'task:TASK-42', resource: 'node-build', state: 'active' }),
      makeGrant({ id: 'rule', scope: 'repo', session_id: 'repo:/work/app', resource: '~/.gradle', state: 'active' }),
      makeGrant({ id: 'a', state: 'active', decided_by: 'policy' }),
      makeGrant({ id: 'w', state: 'pending' }),
      makeGrant({ id: 'r', state: 'revoked', decision_note: 'Stopped by owner' }),
      makeGrant({ id: 'e', state: 'expired' }),
      makeGrant({ id: 'x', state: 'refused', decision_note: 'Credential directory' }),
    ]

    expect(isStandingRule(grants[1])).toBe(true)
    expect(isStandingRule(grants[2])).toBe(false)
    expect(activeResourceGrants(grants).map((g) => g.id)).toEqual(['a'])
    expect(pendingResourceGrants(grants).map((g) => g.id)).toEqual(['w'])
    expect(historyResourceGrants(grants).map((g) => g.id)).toEqual(['r', 'e', 'x'])
    expect(taskProfileFromGrants(grants, 'TASK-42')).toBe('node-build')
    expect(taskProfileFromGrants(grants, 'TASK-99')).toBe('none')
  })

  test('formats standing rules in plain English without expanding tilde', () => {
    expect(
      formatStandingRule({
        scope: 'repo',
        session_id: 'repo:/work/app',
        task_id: null,
        repo: '',
        capability: 'fs_write',
        resource: '~/.gradle',
      }),
    ).toBe('Every helper in /work/app may write to ~/.gradle')

    expect(
      formatStandingRule({
        scope: 'repo',
        session_id: 'repo:/work/app',
        task_id: null,
        repo: '/work/app',
        capability: 'fs_read',
        resource: '~/Library/Android/sdk',
      }),
    ).toBe('Every helper in /work/app may read ~/Library/Android/sdk')

    expect(
      formatStandingRule({
        scope: 'task',
        session_id: 'task:TASK-42',
        task_id: null,
        repo: '',
        capability: 'loopback',
        resource: '5037',
      }),
    ).toBe('Every helper on task TASK-42 may connect to local port 5037')

    expect(
      formatStandingRule({
        scope: 'task',
        session_id: 'task:TASK-42',
        task_id: 'TASK-42',
        repo: '',
        capability: 'egress',
        resource: 'registry.example.org:443',
      }),
    ).toBe('Every helper on task TASK-42 may reach registry.example.org:443')
  })

  test('formats the weekly summary line', () => {
    expect(
      formatSandboxSummary(7, {
        approved_by_policy: 12,
        approved_by_owner: 2,
        asked_owner: 3,
        declined: 1,
        refused: 1,
        standing_rules: 2,
      }),
    ).toBe('In the last 7 days HQ allowed 12 requests for you, asked you 3, and refused 1.')

    expect(
      formatSandboxSummary(7, {
        approved_by_policy: 1,
        approved_by_owner: 0,
        asked_owner: 0,
        declined: 0,
        refused: 0,
        standing_rules: 1,
      }),
    ).toBe('In the last 7 days HQ allowed 1 request for you, asked you 0, and refused 0.')
  })

  test('parses sandbox-grants notification and enriches with matching grant repo and starter', () => {
    const n = makeNotif()
    expect(isSandboxGrantNotification(n)).toBe(true)

    const parsedOnly = parseSandboxGrantNotification(n)
    expect(parsedOnly.sessionId).toBe('hs-1')
    expect(parsedOnly.capability).toBe('fs_write')
    expect(parsedOnly.resource).toBe('/home/user/.gradle')
    expect(parsedOnly.helperReason).toBe('Need gradle cache for build')
    expect(parsedOnly.taskId).toBeNull()
    expect(parsedOnly.repo).toBe('')
    expect(parsedOnly.suggestedStarter).toBeNull()

    const enriched = parseSandboxGrantNotification(n, [makeGrant()])
    expect(enriched.taskId).toBe('TASK-42')
    expect(enriched.repo).toBe('/work/app')
    expect(enriched.suggestedStarter).toBe('mobile-dev')
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
