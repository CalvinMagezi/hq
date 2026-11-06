import { useEffect } from 'react'
import { Link } from '@tanstack/react-router'
import { CheckSquare, ExternalLink, FileText, GitCommit, GitPullRequest, Loader2, MessageSquare, Terminal } from 'lucide-react'
import type { ReactNode } from 'react'
import { fetchTaskLinksClient, type LinkKind, type TaskItem, type TaskLinkItem } from '~/lib/tasksApi'
import { usePolled } from '../sessions/usePolled'
import { SectionLabel } from './taskFields'
import { linkTarget, linkText } from './linkTarget'
import { useRefreshOn } from '~/lib/useRefreshOn'
import './leaf.css'

const LINKS_POLL_MS = 300_000
const ICON_CLASS = 'a-ic xs a-muted'
const ROW_CLASS =
  'a-drow l-tall l-link a-body-sm'

const ICONS: Record<LinkKind, ReactNode> = {
  vault_note: <FileText className={ICON_CLASS} aria-hidden="true" />,
  chat_thread: <MessageSquare className={ICON_CLASS} aria-hidden="true" />,
  session: <Terminal className={ICON_CLASS} aria-hidden="true" />,
  commit: <GitCommit className={ICON_CLASS} aria-hidden="true" />,
  pr: <GitPullRequest className={ICON_CLASS} aria-hidden="true" />,
  url: <ExternalLink className={ICON_CLASS} aria-hidden="true" />,
  task: <CheckSquare className={ICON_CLASS} aria-hidden="true" />,
}

const DIRECTION_NOTE: Record<TaskLinkItem['direction'], string> = {
  origin: 'came from',
  produced: 'produced',
  related: '',
}

function LinkRow({ link }: { link: TaskLinkItem }) {
  const target = linkTarget(link)
  const body = (
    <>
      <span className="mt-0.5">{ICONS[link.kind]}</span>
      <span className="l-grow l-break">
        {DIRECTION_NOTE[link.direction] && <span className="a-muted">{DIRECTION_NOTE[link.direction]} </span>}
        {linkText(link)}
      </span>
    </>
  )
  switch (target.kind) {
    case 'app':
      if (target.route === 'vault') return <Link to="/notes/$" params={{ _splat: target.value }} className={ROW_CLASS}>{body}</Link>
      if (target.route === 'sessions') return <Link to="/helpers/$id" params={{ id: target.value }} className={ROW_CLASS}>{body}</Link>
      return <Link to="/tasks/$id" params={{ id: target.value }} className={ROW_CLASS}>{body}</Link>
    case 'chat':
      // The chat opens the thread named in the address when it mounts.
      return <a href={`/chat/${encodeURIComponent(target.thread)}`} className={ROW_CLASS}>{body}</a>
    case 'web':
      return <a href={target.href} target="_blank" rel="noopener noreferrer" className={ROW_CLASS}>{body}</a>
    case 'none':
      return <div className={ROW_CLASS}>{body}</div>
  }
}

/** What a task came from and produced: notes, chats, sessions, commits, pull requests, links and tasks. */
export function TaskLinks({ task }: { task: TaskItem }) {
  const links = usePolled(task.id, async () => (await fetchTaskLinksClient(task.id)).links, LINKS_POLL_MS, true, true, ['task:sync'])
  useRefreshOn(['task:sync'], links.refresh)
  const { refresh } = links
  useEffect(() => {
    void refresh()
  }, [task.updated_at, refresh])

  if (links.loading) {
    return (
      <section aria-label="Links">
        <SectionLabel>Links</SectionLabel>
        <div role="status" className="a-loading">
          <Loader2 className="a-ic xs animate-spin" aria-hidden="true" />
          Loading links
        </div>
      </section>
    )
  }
  if (!links.data) {
    return (
      <section aria-label="Links">
        <SectionLabel>Links</SectionLabel>
        <p role="alert" className="a-body-sm a-danger-t">{links.error ?? 'Could not load links.'}</p>
      </section>
    )
  }
  if (links.data.length === 0) return null
  return (
    <section aria-label="Links">
      <SectionLabel>Links</SectionLabel>
      <ul className="a-card l-lines">
        {links.data.map((link) => (
          <li key={link.id}>
            <LinkRow link={link} />
          </li>
        ))}
      </ul>
    </section>
  )
}
