import { useEffect } from 'react'
import { Link } from '@tanstack/react-router'
import { CheckSquare, ExternalLink, FileText, GitCommit, GitPullRequest, Loader2, MessageSquare, Terminal } from 'lucide-react'
import type { ReactNode } from 'react'
import { fetchTaskLinksClient, type LinkKind, type TaskItem, type TaskLinkItem } from '~/lib/tasksApi'
import { usePolled } from '../sessions/usePolled'
import { SectionLabel } from './taskFields'
import { linkTarget, linkText } from './linkTarget'
import { useRefreshOn } from '~/lib/useRefreshOn'

const LINKS_POLL_MS = 300_000
const ICON_CLASS = 'w-3.5 h-3.5 text-neutral-500 shrink-0'
const ROW_CLASS =
  'flex items-start gap-2 px-2.5 py-1.5 rounded-lg bg-white/[0.02] border border-white/5 hover:bg-white/5 text-xs font-mono text-neutral-200 min-w-0'

const ICONS: Record<LinkKind, ReactNode> = {
  vault_note: <FileText className={ICON_CLASS} />,
  chat_thread: <MessageSquare className={ICON_CLASS} />,
  session: <Terminal className={ICON_CLASS} />,
  commit: <GitCommit className={ICON_CLASS} />,
  pr: <GitPullRequest className={ICON_CLASS} />,
  url: <ExternalLink className={ICON_CLASS} />,
  task: <CheckSquare className={ICON_CLASS} />,
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
      <span className="min-w-0 break-words">
        {DIRECTION_NOTE[link.direction] && <span className="text-neutral-500">{DIRECTION_NOTE[link.direction]} </span>}
        {linkText(link)}
      </span>
    </>
  )
  switch (target.kind) {
    case 'app':
      if (target.route === 'vault') return <Link to="/vault/$" params={{ _splat: target.value }} className={ROW_CLASS}>{body}</Link>
      if (target.route === 'sessions') return <Link to="/sessions" search={{ id: target.value }} className={ROW_CLASS}>{body}</Link>
      return <Link to="/tasks" search={{ task: target.value }} className={ROW_CLASS}>{body}</Link>
    case 'chat':
      // The chat opens the thread named in the address when it mounts.
      return <a href={`/chat?thread=${encodeURIComponent(target.thread)}`} className={ROW_CLASS}>{body}</a>
    case 'web':
      return <a href={target.href} target="_blank" rel="noopener noreferrer" className={ROW_CLASS}>{body}</a>
    case 'none':
      return <div className={ROW_CLASS}>{body}</div>
  }
}

/** What a task came from and produced: notes, chats, sessions, commits, pull requests, links and tasks. */
export function TaskLinks({ task }: { task: TaskItem }) {
  const links = usePolled(task.id, async () => (await fetchTaskLinksClient(task.id)).links, LINKS_POLL_MS)
  useRefreshOn(['task:sync'], links.refresh)
  const { refresh } = links
  useEffect(() => {
    void refresh()
  }, [task.updated_at, refresh])

  if (links.loading) {
    return (
      <section aria-label="Links">
        <SectionLabel>Links</SectionLabel>
        <div role="status" className="flex items-center gap-2 text-xs font-mono text-neutral-500">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Loading links
        </div>
      </section>
    )
  }
  if (!links.data) {
    return (
      <section aria-label="Links">
        <SectionLabel>Links</SectionLabel>
        <p role="alert" className="text-xs font-mono text-rose-400">{links.error ?? 'Could not load links.'}</p>
      </section>
    )
  }
  if (links.data.length === 0) return null
  return (
    <section aria-label="Links">
      <SectionLabel>Links</SectionLabel>
      <ul className="space-y-1.5">
        {links.data.map((link) => (
          <li key={link.id}>
            <LinkRow link={link} />
          </li>
        ))}
      </ul>
    </section>
  )
}
