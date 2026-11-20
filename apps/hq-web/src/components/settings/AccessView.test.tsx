import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { AccessView } from './AccessView'
import type { SandboxGrant, SandboxGrantProfile, SandboxSummaryResponse } from '~/lib/sandboxGrantsApi'
import { t } from '~/strings'

const html = (el: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(el)

const grant = (over: Partial<SandboxGrant> = {}): SandboxGrant => ({
  id: 'g-1',
  session_id: 'hs-1',
  task_id: 'TASK-1',
  host: 'local',
  repo: '/work/app',
  capability: 'fs_write',
  resource: '/home/user/.gradle',
  state: 'active',
  decided_by: 'policy',
  helper_reason: '',
  decision_note: '',
  requested_at: Date.UTC(2026, 10, 19, 10) / 1000,
  decided_at: Date.UTC(2026, 10, 19, 10) / 1000,
  applied_at: Date.UTC(2026, 10, 19, 10) / 1000,
  expires_at: Date.UTC(2026, 10, 20, 10) / 1000,
  revoked_at: null,
  scope: 'session',
  ...over,
})

const profile: SandboxGrantProfile = {
  name: 'mobile-dev',
  writable: ['/home/user/.gradle', '/home/user/.pub-cache'],
  readable: ['/home/user/sdk'],
  loopback_ports: [5037],
  egress: ['packages.example.org'],
}

const summary: SandboxSummaryResponse = {
  days: 7,
  summary: {
    approved_by_policy: 12,
    approved_by_owner: 2,
    asked_owner: 3,
    declined: 1,
    refused: 1,
    standing_rules: 2,
  },
}

describe('AccessView (Settings > Access)', () => {
  test('renders empty states when no grants, rules or profiles exist', () => {
    const out = html(
      createElement(AccessView, {
        grants: [],
        profiles: [],
        rules: [],
        canChange: true,
        revokingId: null,
        onRevoke: () => {},
      }),
    )
    expect(out).toContain(t('grants_settings_rules_empty'))
    expect(out).toContain(t('grants_settings_active_empty'))
    expect(out).toContain(t('grants_settings_profiles_empty'))
    expect(out).toContain(t('grants_settings_history_empty'))
    expect(out).not.toContain('data-testid="access-summary"')
  })

  test('renders summary line, plain-English standing rules keeping ~ verbatim, active grants, profiles and history', () => {
    const now = Date.UTC(2026, 10, 20, 12)
    const repoRule = grant({
      id: 'rule-1',
      session_id: 'repo:/work/app',
      task_id: null,
      repo: '',
      scope: 'repo',
      capability: 'fs_write',
      resource: '~/.gradle',
      decided_by: 'owner',
      decision_note: 'starter mobile-dev',
      expires_at: null,
    })
    const taskRule = grant({
      id: 'rule-2',
      session_id: 'task:TASK-9',
      task_id: null,
      repo: '',
      scope: 'task',
      capability: 'egress',
      resource: 'registry.example.org:443',
      decided_by: 'owner',
      decision_note: 'allowed for this task',
      expires_at: null,
    })

    const out = html(
      createElement(AccessView, {
        grants: [
          repoRule,
          grant({ id: 'g-active', state: 'active', decided_by: 'policy', resource: '/home/user/.gradle' }),
          grant({ id: 'g-refused', state: 'refused', resource: '/home/user/.ssh', decision_note: 'Credential store' }),
        ],
        rules: [repoRule, taskRule],
        summary,
        profiles: [profile],
        canChange: true,
        revokingId: null,
        onRevoke: () => {},
        nowMs: now,
      }),
    )
    expect(out).toContain('data-testid="access-summary"')
    expect(out).toContain('In the last 7 days HQ allowed 12 requests for you, asked you 3, and refused 1.')
    expect(out).toContain('data-testid="access-standing-rule"')
    expect(out).toContain('Every helper in /work/app may write to ~/.gradle')
    expect(out).toContain('Every helper on task TASK-9 may reach registry.example.org:443')
    // Standing rules do not duplicate in Active grants.
    expect(out.match(/data-testid="access-active-grant"/g)).toHaveLength(1)
    expect(out).toContain('/home/user/.gradle')
    expect(out).toContain(t('grants_revoke'))
    expect(out).toContain('mobile-dev')
    expect(out).toContain('/home/user/.pub-cache')
    expect(out).toContain('/home/user/sdk')
    expect(out).toContain('5037')
    expect(out).toContain('packages.example.org')
    expect(out).toContain('data-testid="access-history-row"')
    expect(out).toContain('/home/user/.ssh')
    expect(out).toContain('Credential store')
  })

  test('disables Revoke when disconnected and shows error alert', () => {
    const out = html(
      createElement(AccessView, {
        grants: [grant()],
        profiles: [],
        canChange: false,
        revokingId: null,
        error: 'Could not revoke grant.',
        onRevoke: () => {},
      }),
    )
    expect(out).toContain('disabled')
    expect(out).toContain(t('settings_folders_revoke_needs').replace("'", '&#x27;'))
    expect(out).toContain('role="alert"')
    expect(out).toContain('Could not revoke grant.')
  })
})
