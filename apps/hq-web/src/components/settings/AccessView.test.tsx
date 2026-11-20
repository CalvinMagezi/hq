import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { AccessView } from './AccessView'
import type { SandboxGrant, SandboxGrantProfile } from '~/lib/sandboxGrantsApi'
import { t } from '~/strings'

const html = (el: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(el)

const grant = (over: Partial<SandboxGrant> = {}): SandboxGrant => ({
  id: 'g-1',
  session_id: 'hs-1',
  task_id: 'TASK-1',
  host: 'local',
  repo: '/home/user/project',
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
  ...over,
})

const profile: SandboxGrantProfile = {
  name: 'mobile-dev',
  writable: ['/home/user/.gradle', '/home/user/.pub-cache'],
  readable: ['/home/user/sdk'],
  loopback_ports: [5037],
  egress: ['packages.example.org'],
}

describe('AccessView (Settings > Access)', () => {
  test('renders empty states when no grants or profiles exist', () => {
    const out = html(
      createElement(AccessView, {
        grants: [],
        profiles: [],
        canChange: true,
        revokingId: null,
        onRevoke: () => {},
      }),
    )
    expect(out).toContain(t('grants_settings_active_empty'))
    expect(out).toContain(t('grants_settings_profiles_empty'))
    expect(out).toContain(t('grants_settings_history_empty'))
  })

  test('renders active grants with Revoke, read-only profiles and history with decision_note', () => {
    const now = Date.UTC(2026, 10, 20, 12)
    const out = html(
      createElement(AccessView, {
        grants: [
          grant({ id: 'g-active', state: 'active', decided_by: 'policy', resource: '/home/user/.gradle' }),
          grant({ id: 'g-refused', state: 'refused', resource: '/home/user/.ssh', decision_note: 'Credential store' }),
        ],
        profiles: [profile],
        canChange: true,
        revokingId: null,
        onRevoke: () => {},
        nowMs: now,
      }),
    )
    expect(out).toContain('data-testid="access-active-grant"')
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
