import type { ReactNode } from 'react'
import { Link } from '@tanstack/react-router'
import { awayLinkLine } from '~/lib/tasksAway'
import { fetchBriefing } from '~/lib/verificationApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../sessions/usePolled'
import { verificationActive } from './verification/verification'

const LINK_POLL_MS = 120_000
const LINK_HOURS = 24

/** The list always comes first; the checking blocks follow it, so a phone shows tasks before anything else. */
export function TasksBody({ list, checking }: { list: ReactNode; checking: ReactNode }) {
  return (
    <>
      {list}
      {checking}
    </>
  )
}

/** One line under the list that opens the away page, once HQ has finished something. */
export function AwayLinkRow() {
  const found = usePolled('verification-briefing', () => fetchBriefing(LINK_HOURS), LINK_POLL_MS, true, true, ['task:sync'])
  useRefreshOn(['task:sync'], found.refresh)
  const b = found.data
  const line = b && verificationActive(b.mode) ? awayLinkLine(b) : null
  if (!line) return null
  return (
    <Link to="/tasks/away" className="a-rcard a-pad">
      {line}
    </Link>
  )
}
