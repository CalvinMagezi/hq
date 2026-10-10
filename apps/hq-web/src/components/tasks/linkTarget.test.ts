import { expect, test } from 'bun:test'
import { linkTarget, linkText } from './linkTarget'

const link = (kind: Parameters<typeof linkTarget>[0]['kind'], ref: string) => ({ kind, ref })

test('a note, session, task and chat thread lead inside the app', () => {
  expect(linkTarget(link('vault_note', 'Notebooks/plan.md'))).toEqual({ kind: 'app', route: 'vault', value: 'Notebooks/plan.md' })
  expect(linkTarget(link('session', 'hs-1'))).toEqual({ kind: 'app', route: 'sessions', value: 'hs-1' })
  expect(linkTarget(link('task', 'tk-1'))).toEqual({ kind: 'app', route: 'tasks', value: 'tk-1' })
  expect(linkTarget(link('chat_thread', 'thr-9'))).toEqual({ kind: 'chat', thread: 'thr-9' })
})

test('a pull request and a commit with a repository lead to GitHub', () => {
  expect(linkTarget(link('pr', 'owner/repo#12'))).toEqual({ kind: 'web', href: 'https://github.com/owner/repo/pull/12' })
  expect(linkTarget(link('commit', 'owner/repo@abcdef1'))).toEqual({
    kind: 'web',
    href: 'https://github.com/owner/repo/commit/abcdef1',
  })
})

test('a bare commit sha has nowhere to go', () => {
  expect(linkTarget(link('commit', 'abcdef1'))).toEqual({ kind: 'none' })
})

test('only http and https are ever opened', () => {
  expect(linkTarget(link('url', 'https://example.com/a'))).toEqual({ kind: 'web', href: 'https://example.com/a' })
  for (const bad of ['javascript:alert(1)', 'data:text/html,x', 'file:///etc/passwd', 'not a url', '']) {
    expect(linkTarget(link('url', bad))).toEqual({ kind: 'none' })
  }
})

test('a malformed pull request ref is not turned into a URL', () => {
  expect(linkTarget(link('pr', '../evil#1'))).toEqual({ kind: 'none' })
  expect(linkTarget(link('pr', 'owner/repo#x'))).toEqual({ kind: 'none' })
})

test('link text prefers the label, then the target, then the ref', () => {
  expect(linkText({ kind: 'url', ref: 'https://x.io', label: 'the spec' })).toBe('the spec')
  expect(linkText({ kind: 'vault_note', ref: 'Notebooks/Projects/plan.md', label: '' })).toBe('plan')
  expect(
    linkText({
      kind: 'task',
      ref: 'tk-1',
      label: '',
      linked_task: { id: 'tk-1', display_id: 'FR-1', title: 'Other', status: 'to_do' },
    })
  ).toBe('FR-1 Other')
})
