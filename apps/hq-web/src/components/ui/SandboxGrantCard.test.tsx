import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { SandboxGrantCard } from './SandboxGrantCard'
import type { ParsedSandboxGrant, SandboxGrantProfile } from '~/lib/sandboxGrantsApi'
import { t } from '~/strings'

const html = (el: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(el)

const baseGrant = (over: Partial<ParsedSandboxGrant> = {}): ParsedSandboxGrant => ({
  title: 'A helper wants to write to /home/user/.gradle',
  sessionId: 'hs-1',
  taskId: 'TASK-7',
  repo: '/work/app',
  capability: 'fs_write',
  resource: '/home/user/.gradle',
  helperReason: 'Needs cache to compile project',
  suggestedStarter: 'mobile-dev',
  ...over,
})

const starters: SandboxGrantProfile[] = [
  {
    name: 'mobile-dev',
    writable: ['~/.gradle', '~/.pub-cache'],
    readable: ['~/Library/Android/sdk'],
    loopback_ports: [5037],
    egress: ['packages.example.org'],
  },
  {
    name: 'node-build',
    writable: ['~/.npm'],
    readable: [],
    loopback_ports: [],
    egress: ['registry.example.org'],
  },
  {
    name: 'rust-build',
    writable: ['~/.cargo'],
    readable: [],
    loopback_ports: [],
    egress: ['crates.example.org'],
  },
]

describe('SandboxGrantCard', () => {
  test('orders safe choices first, labels helper words, and shows starter contents beneath the starter choice', () => {
    const out = html(
      createElement(SandboxGrantCard, {
        grant: baseGrant(),
        starters,
        onAnswer: () => {},
      }),
    )
    expect(out).toContain('A helper wants to write to /home/user/.gradle')
    expect(out).toContain(t('grants_mode_fs_write'))
    expect(out).toContain('/home/user/.gradle')
    expect(out).toContain('TASK-7')
    expect(out).toContain('/work/app')
    expect(out).toContain(t('grants_expires_day'))
    expect(out).toContain(t('grants_helper_words').replace("'", '&#x27;').replace("'", '&#x27;'))
    expect(out).toContain('Needs cache to compile project')

    const onceAt = out.indexOf(t('grants_allow_once'))
    const taskAt = out.indexOf(t('grants_allow_task'))
    const repoAt = out.indexOf(t('grants_allow_repo'))
    const starterAt = out.indexOf(t('grants_allow_starter_mobile_dev'))
    const includesAt = out.indexOf('data-testid="grant-starter-includes"')
    const declineAt = out.indexOf(t('grants_decline'))

    expect(onceAt).toBeGreaterThan(-1)
    expect(taskAt).toBeGreaterThan(onceAt)
    expect(repoAt).toBeGreaterThan(taskAt)
    expect(starterAt).toBeGreaterThan(repoAt)
    expect(includesAt).toBeGreaterThan(starterAt)
    expect(declineAt).toBeGreaterThan(includesAt)

    expect(out).toContain(t('grants_starter_mobile_dev'))
    expect(out).toContain('~/.gradle')
    expect(out).toContain('~/.pub-cache')
    expect(out).toContain('~/Library/Android/sdk')
    expect(out).toContain('5037')
    expect(out).toContain('packages.example.org')
    expect(out.match(/<button /g)).toHaveLength(5)
  })

  test('renders Node tools and Rust tools starter choices cleanly', () => {
    const nodeOut = html(
      createElement(SandboxGrantCard, {
        grant: baseGrant({ suggestedStarter: 'node-build' }),
        starters,
        onAnswer: () => {},
      }),
    )
    expect(nodeOut).toContain(t('grants_allow_starter_node_build'))
    expect(nodeOut).toContain(t('grants_starter_node_build'))
    expect(nodeOut).toContain('~/.npm')
    expect(nodeOut).toContain('registry.example.org')

    const rustOut = html(
      createElement(SandboxGrantCard, {
        grant: baseGrant({ suggestedStarter: 'rust-build' }),
        starters,
        onAnswer: () => {},
      }),
    )
    expect(rustOut).toContain(t('grants_allow_starter_rust_build'))
    expect(rustOut).toContain(t('grants_starter_rust_build'))
    expect(rustOut).toContain('~/.cargo')
    expect(rustOut).toContain('crates.example.org')
  })

  test('omits task, repo, starter and helper words when absent, and hides title when showTitle is false', () => {
    const out = html(
      createElement(SandboxGrantCard, {
        grant: baseGrant({
          capability: 'egress',
          resource: 'api.example.org:443',
          taskId: null,
          repo: '',
          helperReason: '',
          suggestedStarter: null,
        }),
        showTitle: false,
        onAnswer: () => {},
      }),
    )
    expect(out).toContain(t('grants_mode_egress'))
    expect(out).toContain('api.example.org:443')
    expect(out).not.toContain(t('grants_allow_task'))
    expect(out).not.toContain(t('grants_allow_repo'))
    expect(out).not.toContain('data-testid="grant-starter-choice"')
    expect(out).not.toContain('data-testid="grant-helper-reason"')
    expect(out).not.toContain('<p class="text-xs font-semibold">')
    expect(out.match(/<button /g)).toHaveLength(2)
  })

  test('renders busy and server error states', () => {
    const out = html(
      createElement(SandboxGrantCard, {
        grant: baseGrant({ capability: 'loopback', resource: '5037', suggestedStarter: null }),
        busy: true,
        error: 'this request is not on a task, so it cannot be allowed for one',
        onAnswer: () => {},
      }),
    )
    expect(out).toContain(t('grants_mode_loopback'))
    expect(out).toContain('disabled')
    expect(out).toContain(t('approval_answering'))
    expect(out).toContain('role="alert"')
    expect(out).toContain('this request is not on a task, so it cannot be allowed for one')
  })
})
