import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { AccessProfilePickerView } from './AccessProfilePicker'
import type { SandboxGrantProfile } from '~/lib/sandboxGrantsApi'
import { t } from '~/strings'

const html = (el: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(el)

const profiles: SandboxGrantProfile[] = [
  {
    name: 'node-build',
    writable: ['/home/user/.npm'],
    readable: ['/home/user/tools'],
    loopback_ports: [5173],
    egress: ['registry.example.org'],
  },
  {
    name: 'empty-profile',
    writable: [],
    readable: [],
    loopback_ports: [],
    egress: [],
  },
]

describe('AccessProfilePickerView', () => {
  test('renders None plus profiles with no preview when None is selected', () => {
    const out = html(
      createElement(AccessProfilePickerView, {
        profiles,
        value: 'none',
        onChange: () => {},
      }),
    )
    expect(out).toContain(t('grants_allow_dev_access'))
    expect(out).toContain(t('grants_profile_none'))
    expect(out).toContain('node-build')
    expect(out).not.toContain('data-testid="profile-preview"')
  })

  test('shows exact capabilities allowed beside selector when a profile is selected', () => {
    const out = html(
      createElement(AccessProfilePickerView, {
        profiles,
        value: 'node-build',
        status: t('grants_profile_saved'),
        onChange: () => {},
      }),
    )
    expect(out).toContain('data-testid="profile-preview"')
    expect(out).toContain('/home/user/.npm')
    expect(out).toContain('/home/user/tools')
    expect(out).toContain('5173')
    expect(out).toContain('registry.example.org')
    expect(out).toContain(t('grants_profile_saved'))
  })

  test('shows empty note for a profile with no capabilities and renders error state', () => {
    const out = html(
      createElement(AccessProfilePickerView, {
        profiles,
        value: 'empty-profile',
        disabled: true,
        error: 'Could not update access profile.',
        onChange: () => {},
      }),
    )
    expect(out).toContain(t('grants_profile_empty'))
    expect(out).toContain('disabled')
    expect(out).toContain('role="alert"')
    expect(out).toContain('Could not update access profile.')
  })
})
