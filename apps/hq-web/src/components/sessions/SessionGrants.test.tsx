import { describe, expect, test } from 'bun:test'
import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from '@tanstack/react-router'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { BlockedCallout } from './BlockedCallout'
import { SessionGrantsTimeline } from './SessionGrantsTimeline'
import type { HarnessSession } from '~/lib/sessionsApi'
import type { SandboxGrant } from '~/lib/sandboxGrantsApi'
import { t } from '~/strings'

async function inRouter(node: () => ReturnType<typeof createElement>): Promise<string> {
  const root = createRootRoute({ component: node })
  const router = createRouter({ routeTree: root, history: createMemoryHistory({ initialEntries: ['/'] }) })
  await router.load()
  return renderToStaticMarkup(createElement(RouterProvider, { router }))
}

const makeSession = (over: Partial<HarnessSession> = {}): HarnessSession => ({
  id: 'hs-1',
  harness: 'claude-code',
  label: 'demo-app',
  host: 'local',
  cwd: '/home/user/demo-app',
  status: 'running',
  agent_status: 'blocked',
  last_seen_at: '2026-11-20 08:00:00',
  drive: false,
  mode: 'observe',
  goal: null,
  done_criteria: null,
  drive_blocked_by: [],
  drive_off_reason: null,
  nudges_sent: 0,
  pending_wake: null,
  last_driven_at: null,
  created_at: '2026-11-20 07:50:00',
  task: null,
  agent_name: 'demo-app',
  owner_thread: null,
  archived: false,
  ...over,
})

const makeGrant = (over: Partial<SandboxGrant> = {}): SandboxGrant => ({
  id: 'g-1',
  session_id: 'hs-1',
  task_id: null,
  host: 'local',
  repo: '/home/user/demo-app',
  capability: 'fs_write',
  resource: '/home/user/.cache/build',
  state: 'pending',
  decided_by: 'system',
  helper_reason: 'Need build cache',
  decision_note: '',
  requested_at: 1_700_000_000,
  decided_at: null,
  applied_at: null,
  expires_at: 1_700_086_400,
  revoked_at: null,
  ...over,
})

describe('BlockedCallout with pending sandbox grant', () => {
  test('shows Waiting for access instead of generic blocked prompt', async () => {
    const out = await inRouter(() =>
      createElement(BlockedCallout, {
        session: makeSession(),
        screenText: 'generic terminal text',
        pendingGrant: makeGrant(),
        onSent: () => {},
      }),
    )
    expect(out).toContain(t('grants_waiting_for', { resource: '/home/user/.cache/build' }))
    expect(out).toContain(t('grants_waiting_inbox'))
    expect(out).toContain('Need build cache')
    expect(out).not.toContain(t('helpers_unreadable'))
  })

  test('falls back to generic blocked prompt when no pending grant exists', async () => {
    const out = await inRouter(() =>
      createElement(BlockedCallout, {
        session: makeSession(),
        screenText: 'waiting on prompt',
        pendingGrant: null,
        onSent: () => {},
      }),
    )
    expect(out).toContain(t('helpers_waiting_for_you'))
    expect(out).not.toContain('Waiting for access:')
  })
})

describe('SessionGrantsTimeline', () => {
  test('renders quiet HQ allowed row with Revoke for policy grants and muted rows with decision_note', () => {
    const grants: SandboxGrant[] = [
      makeGrant({ id: 'g-pol', state: 'active', decided_by: 'policy', resource: '/home/user/.cargo' }),
      makeGrant({ id: 'g-rev', state: 'revoked', resource: '/home/user/.npm', decision_note: 'Revoked by owner' }),
      makeGrant({ id: 'g-ref', state: 'refused', resource: '/home/user/.ssh', decision_note: 'Credential directory' }),
      makeGrant({ id: 'g-exp', state: 'expired', resource: 'api.example.org:443', decision_note: '' }),
      makeGrant({ id: 'g-pend', state: 'pending', resource: '/home/user/other' }),
      makeGrant({ id: 'g-prof', capability: 'profile', state: 'active', resource: 'rust-build' }),
    ]
    const out = renderToStaticMarkup(
      createElement(SessionGrantsTimeline, {
        grants,
        revokingId: null,
        onRevoke: () => {},
      }),
    )
    expect(out).toContain(t('grants_hq_allowed', { resource: '/home/user/.cargo' }))
    expect(out).toContain(t('grants_revoke'))
    expect(out).toContain('Revoked by owner')
    expect(out).toContain('Credential directory')
    expect(out).toContain('api.example.org:443')
    // Pending grants and profile bindings are excluded from timeline rows.
    expect(out).not.toContain('/home/user/other')
    expect(out).not.toContain('rust-build')
  })

  test('renders nothing when there are no settled or active resource grants', () => {
    const out = renderToStaticMarkup(
      createElement(SessionGrantsTimeline, {
        grants: [makeGrant({ state: 'pending' })],
        revokingId: null,
        onRevoke: () => {},
      }),
    )
    expect(out).toBe('')
  })

  test('shows busy label and error alert when revoking', () => {
    const out = renderToStaticMarkup(
      createElement(SessionGrantsTimeline, {
        grants: [makeGrant({ id: 'g-pol', state: 'active', decided_by: 'policy' })],
        revokingId: 'g-pol',
        error: 'Could not revoke grant.',
        onRevoke: () => {},
      }),
    )
    expect(out).toContain(t('grants_revoking'))
    expect(out).toContain('role="alert"')
    expect(out).toContain('Could not revoke grant.')
  })
})
