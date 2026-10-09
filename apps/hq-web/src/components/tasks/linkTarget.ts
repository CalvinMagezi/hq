import type { TaskLinkItem } from '~/lib/tasksApi'

/** Where a link leads: inside this app, out to the web, or nowhere. */
export type LinkTarget =
  | { kind: 'app'; route: 'vault' | 'sessions' | 'tasks'; value: string }
  | { kind: 'chat'; thread: string }
  | { kind: 'web'; href: string }
  | { kind: 'none' }

const SHA_PATTERN = /^[0-9a-f]{7,40}$/
const REPO_PATTERN = /^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/

/** `owner/repo` with neither part being `.` or `..`, which the browser would resolve to another repository. */
function isRepo(value: string | undefined): value is string {
  return value !== undefined && REPO_PATTERN.test(value) && value.split('/').every((part) => part !== '.' && part !== '..')
}

/** Only a real web address is ever opened: a stored ref is data, not a command. */
function webAddress(ref: string): string | null {
  try {
    const url = new URL(ref)
    return url.protocol === 'https:' || url.protocol === 'http:' ? url.toString() : null
  } catch {
    return null
  }
}

export function linkTarget(link: Pick<TaskLinkItem, 'kind' | 'ref' | 'linked_task'>): LinkTarget {
  switch (link.kind) {
    case 'vault_note':
      return { kind: 'app', route: 'vault', value: link.ref }
    case 'chat_thread':
      return { kind: 'chat', thread: link.ref }
    case 'session':
      return { kind: 'app', route: 'sessions', value: link.ref }
    case 'task':
      return { kind: 'app', route: 'tasks', value: link.ref }
    case 'url': {
      const href = webAddress(link.ref)
      return href ? { kind: 'web', href } : { kind: 'none' }
    }
    case 'pr': {
      const [repo, number] = link.ref.split('#')
      return isRepo(repo) && /^\d+$/.test(number ?? '')
        ? { kind: 'web', href: `https://github.com/${repo}/pull/${number}` }
        : { kind: 'none' }
    }
    case 'commit': {
      const [repo, sha] = link.ref.includes('@') ? link.ref.split('@') : [undefined, link.ref]
      // A bare sha says nothing about which repository it is in.
      return isRepo(repo) && SHA_PATTERN.test(sha ?? '')
        ? { kind: 'web', href: `https://github.com/${repo}/commit/${sha}` }
        : { kind: 'none' }
    }
  }
}

/** A short human label for a link's ref. */
export function linkText(link: Pick<TaskLinkItem, 'kind' | 'ref' | 'label' | 'linked_task'>): string {
  if (link.label) return link.label
  if (link.kind === 'task' && link.linked_task) return `${link.linked_task.display_id} ${link.linked_task.title}`
  if (link.kind === 'vault_note') return link.ref.split('/').pop()?.replace(/\.md$/, '') ?? link.ref
  return link.ref
}
