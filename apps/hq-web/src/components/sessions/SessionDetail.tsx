import { useEffect, useRef, useState } from 'react'
import { Link } from '@tanstack/react-router'
import { ArrowLeft, Check, Copy, Eye, Loader2 } from 'lucide-react'
import { useThreadStore } from '~/store/threadStore'
import { refreshThreads } from '~/lib/threadApi'
import { attachCaveat, attachCommand, canSend, globalSessionsApi, type AdoptResult, type HarnessSession } from '~/lib/sessionsApi'
import { SessionTerminal } from './SessionTerminal'
import { SessionSendBox } from './SessionSendBox'
import { SessionBadges, SessionTaskLink } from './SessionRow'

const COPIED_FLASH_MS = 1500

interface Props {
  session: HarnessSession
  onBack: () => void
  onChanged: () => void
}

/** One session: where it runs, the live terminal text, a send box, Adopt and the attach command. */
export function SessionDetail({ session: s, onBack, onChanged }: Props) {
  const [sentCount, setSentCount] = useState(0)
  return (
    <div className="flex flex-col h-full min-h-0">
      <header className="px-3 py-2 border-b border-white/10 space-y-1.5">
        <div className="flex items-center gap-2 min-w-0">
          <button
            type="button"
            onClick={onBack}
            className="md:hidden flex items-center justify-center h-11 w-11 -ml-2 rounded text-neutral-400 hover:text-white"
            aria-label="Back to sessions"
          >
            <ArrowLeft className="w-4 h-4" />
          </button>
          <h2 className="text-sm font-mono text-neutral-100 truncate min-w-0" title={s.cwd}>
            {s.harness} · {s.label || s.id}
          </h2>
          <span className="flex-1" />
          <AdoptControl session={s} onAdopted={onChanged} />
        </div>
        <SessionBadges session={s} />
        <SessionTaskLink session={s} />
        {s.goal && <p className="text-[11px] font-mono text-neutral-400 line-clamp-3">Goal: {s.goal}</p>}
        <AttachHint session={s} />
      </header>
      <SessionTerminal sessionId={s.id} refreshKey={sentCount} active />
      <SessionSendBox session={s} enabled={canSend(s)} onSent={() => setSentCount((n) => n + 1)} />
    </div>
  )
}

function AdoptControl({ session: s, onAdopted }: { session: HarnessSession; onAdopted: () => void }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState<AdoptResult | null>(null)
  const threadId = done?.thread_id ?? s.owner_thread

  const adopt = async () => {
    setBusy(true)
    setError(null)
    try {
      const result = await globalSessionsApi.adopt(s.id)
      setDone(result)
      await refreshThreads()
      useThreadStore.getState().setActiveThread(result.thread_id)
      onAdopted()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Adopt failed')
    } finally {
      setBusy(false)
    }
  }

  if (threadId) {
    return (
      <Link
        to="/chat"
        onClick={() => useThreadStore.getState().setActiveThread(threadId)}
        className="flex items-center gap-1.5 h-11 sm:h-9 px-3 rounded border border-white/10 text-[11px] font-mono text-neutral-300 hover:text-white hover:bg-white/10 shrink-0"
        title="This session is watched by a chat. Open it."
      >
        <Eye className="w-3.5 h-3.5" />
        Open chat
      </Link>
    )
  }
  return (
    <div className="flex flex-col items-end gap-1 shrink-0">
      <button
        type="button"
        onClick={() => void adopt()}
        disabled={busy}
        className="flex items-center gap-1.5 h-11 sm:h-9 px-3 rounded border border-white/10 text-[11px] font-mono text-neutral-200 hover:bg-white/10 disabled:opacity-50"
        title="Start a chat that watches this session. HQ only observes until you turn Drive on."
      >
        {busy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Eye className="w-3.5 h-3.5" />}
        Adopt
      </button>
      {error && (
        <span role="alert" className="text-[10px] font-mono text-rose-400 max-w-56 text-right">
          {error}
        </span>
      )}
    </div>
  )
}

function AttachHint({ session: s }: { session: HarnessSession }) {
  const [copied, setCopied] = useState(false)
  const resetTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  useEffect(() => () => clearTimeout(resetTimer.current), [])
  const command = attachCommand(s)
  const caveat = attachCaveat(s)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(command)
      setCopied(true)
      clearTimeout(resetTimer.current)
      resetTimer.current = setTimeout(() => setCopied(false), COPIED_FLASH_MS)
    } catch {
      // Clipboard access can be denied; the command stays visible to select by hand.
    }
  }
  return (
    <div className="space-y-0.5">
      <div className="flex items-center gap-2 min-w-0 text-[11px] font-mono text-neutral-500">
        <span className="shrink-0">Attach</span>
        <code className="px-1.5 py-0.5 rounded bg-black/40 text-neutral-300 truncate min-w-0 select-all">{command}</code>
        <button
          type="button"
          onClick={() => void copy()}
          className="flex items-center justify-center h-11 w-11 sm:h-8 sm:w-8 rounded text-neutral-400 hover:text-white hover:bg-white/10 shrink-0"
          aria-label="Copy attach command"
        >
          {copied ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
        </button>
        <span className="hidden sm:inline truncate min-w-0">then open the "hq {s.label || s.harness}" workspace</span>
      </div>
      {caveat && <p className="text-[10px] font-mono text-neutral-500">{caveat}</p>}
    </div>
  )
}
