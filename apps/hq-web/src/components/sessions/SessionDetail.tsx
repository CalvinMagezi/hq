import { useCallback, useState } from 'react'
import { ArrowLeft } from 'lucide-react'
import { attachCommand, canSend, isBlocked, type HarnessSession } from '~/lib/sessionsApi'
import { computerName, folderName, screenPollMs } from '~/lib/workbench'
import { SessionTerminal } from './SessionTerminal'
import { SessionSendBox } from './SessionSendBox'
import { SessionBadges, SessionTaskLink } from './SessionRow'
import { BlockedCallout } from './BlockedCallout'
import { ActionError, ArchiveButton, FollowInChat, RenameTitle, ResumeButton, StopButton, useAction } from './SessionActions'

type Refresh = () => void | Promise<void>

interface Props {
  session: HarnessSession
  onBack: () => void
  onChanged: Refresh
}

/** One agent: where it works, what it is doing now, what it needs from you, and the actions you can take. */
export function SessionDetail({ session: s, onBack, onChanged }: Props) {
  const [sentCount, setSentCount] = useState(0)
  const [screenText, setScreenText] = useState('')
  const action = useAction(onChanged)
  const running = s.status === 'running'
  const blocked = isBlocked(s)
  const folder = folderName(s.cwd)
  const afterSend = useCallback(async () => {
    setSentCount((n) => n + 1)
    await onChanged()
  }, [onChanged])

  const extras = (
    <>
      {s.goal && <p className="text-[11px] font-mono text-neutral-400 line-clamp-3">Goal: {s.goal}</p>}
      <Advanced session={s} />
    </>
  )
  return (
    <div className="flex flex-col h-full min-h-0 overflow-y-auto overscroll-contain">
      <header className="px-3 py-2 border-b border-white/10 space-y-1.5 shrink-0">
        <div className="flex items-center gap-2 min-w-0">
          <button
            type="button"
            onClick={onBack}
            className="md:hidden flex items-center justify-center h-11 w-11 -ml-2 rounded text-neutral-400 hover:text-white"
            aria-label="Back to the Workbench"
          >
            <ArrowLeft className="w-4 h-4" />
          </button>
          <RenameTitle session={s} action={action} />
        </div>
        <p className="text-[11px] font-mono text-neutral-500 truncate">
          {computerName(s.host)}
          {folder && ` · ${folder}`}
        </p>
        <SessionBadges session={s} />
        <SessionTaskLink session={s} />
        <div className="flex flex-wrap items-start gap-1.5">
          {running && <StopButton session={s} action={action} />}
          {!running && <ArchiveButton session={s} action={action} />}
          <FollowInChat session={s} action={action} />
        </div>
        <ActionError error={action.error} />
        {blocked ? (
          <details>
            <summary className="flex items-center min-h-11 cursor-pointer text-[11px] font-mono text-neutral-500 hover:text-neutral-300 select-none">Details</summary>
            {extras}
          </details>
        ) : (
          extras
        )}
      </header>
      <SessionTerminal sessionId={s.id} refreshKey={sentCount} active pollMs={screenPollMs(s)} onText={setScreenText} />
      <Footer session={s} screenText={screenText} onSent={afterSend} onChanged={onChanged} />
    </div>
  )
}

interface FooterProps {
  session: HarnessSession
  screenText: string
  onSent: Refresh
  onChanged: Refresh
}

function Footer({ session: s, screenText, onSent, onChanged }: FooterProps) {
  const resume = useAction(onChanged)
  if (s.status !== 'running') {
    return (
      <section aria-label="Resume" className="shrink-0 border-t border-white/10 px-3 py-3 space-y-2">
        <p className="text-xs font-mono text-neutral-300">This agent has stopped. Resume it to pick up where it left off.</p>
        <ResumeButton session={s} action={resume} />
        <ActionError error={resume.error} />
      </section>
    )
  }
  if (!canSend(s)) {
    return (
      <p className="shrink-0 border-t border-white/10 px-3 py-3 text-xs font-mono" style={{ color: 'var(--accent-amber)' }}>
        HQ cannot reach this agent right now. It will show up again when the computer is back.
      </p>
    )
  }
  if (isBlocked(s)) return <BlockedCallout session={s} screenText={screenText} onSent={onSent} />
  return <SessionSendBox session={s} onSent={onSent} />
}

/** The command for checking this computer from a terminal. Most people never need it. */
function Advanced({ session: s }: { session: HarnessSession }) {
  return (
    <details>
      <summary className="flex items-center min-h-11 cursor-pointer text-[11px] font-mono text-neutral-500 hover:text-neutral-300 select-none">Advanced</summary>
      <div className="space-y-1 pb-1">
        <p className="text-[11px] font-mono text-neutral-500">Check this computer from a terminal</p>
        <code className="block px-1.5 py-1 rounded bg-black/40 text-[11px] font-mono text-neutral-300 break-all select-all">{attachCommand(s)}</code>
      </div>
    </details>
  )
}
