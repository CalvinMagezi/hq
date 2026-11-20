import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { SandboxGrantCard } from './SandboxGrantCard'
import type { ParsedSandboxGrant } from '~/lib/sandboxGrantsApi'
import { t } from '~/strings'

const html = (el: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(el)

const baseGrant = (over: Partial<ParsedSandboxGrant> = {}): ParsedSandboxGrant => ({
  title: 'A helper wants to write to /home/user/.gradle',
  sessionId: 'hs-1',
  taskId: 'TASK-7',
  capability: 'fs_write',
  resource: '/home/user/.gradle',
  helperReason: 'Needs cache to compile project',
  ...over,
})

describe('SandboxGrantCard', () => {
  test('shows plain title, exact resource and mode, task, expiry, helper words and only Allow/Decline', () => {
    const out = html(
      createElement(SandboxGrantCard, {
        grant: baseGrant(),
        onAllow: () => {},
        onDecline: () => {},
      }),
    )
    expect(out).toContain('A helper wants to write to /home/user/.gradle')
    expect(out).toContain(t('grants_mode_fs_write'))
    expect(out).toContain('/home/user/.gradle')
    expect(out).toContain('TASK-7')
    expect(out).toContain(t('grants_expires_day'))
    expect(out).toContain(t('grants_helper_words').replace("'", '&#x27;').replace("'", '&#x27;'))
    expect(out).toContain('Needs cache to compile project')
    expect(out).toContain(t('grants_allow'))
    expect(out).toContain(t('grants_decline'))
    // Only Decline and Allow buttons exist.
    expect(out.match(/<button /g)).toHaveLength(2)
  })

  test('omits task row and helper words block when absent, and hides title when showTitle is false', () => {
    const out = html(
      createElement(SandboxGrantCard, {
        grant: baseGrant({ capability: 'egress', resource: 'api.example.org:443', taskId: null, helperReason: '' }),
        showTitle: false,
        onAllow: () => {},
        onDecline: () => {},
      }),
    )
    expect(out).toContain(t('grants_mode_egress'))
    expect(out).toContain('api.example.org:443')
    expect(out).not.toContain('TASK-7')
    expect(out).not.toContain('data-testid="grant-helper-reason"')
    expect(out).not.toContain('<p class="text-xs font-semibold">')
  })

  test('renders busy and error states', () => {
    const out = html(
      createElement(SandboxGrantCard, {
        grant: baseGrant({ capability: 'loopback', resource: '5037' }),
        busy: true,
        error: 'Request already settled.',
        onAllow: () => {},
        onDecline: () => {},
      }),
    )
    expect(out).toContain(t('grants_mode_loopback'))
    expect(out).toContain('disabled')
    expect(out).toContain(t('approval_answering'))
    expect(out).toContain('role="alert"')
    expect(out).toContain('Request already settled.')
  })
})
