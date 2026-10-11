import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { CheckCircle2, Loader2, OctagonX, X } from 'lucide-react'
import { workbenchApi, type LockdownResult, type WorkbenchHost } from '~/lib/sessionsApi'
import { computerName } from '~/lib/workbench'
import { ErrorText } from './ErrorText'
import { useModal } from './useModal'

const ALL_COMPUTERS = ''
const MAX_REASON_CHARS = 300

interface ButtonProps {
  /** An icon-only button, for the collapsed sidebar. */
  compact?: boolean
  /** Called after a stop went through, so the page can refresh what it shows. */
  onStopped: () => void
}

/** The emergency stop: kills every agent on one computer or all of them, after one confirmation. */
export function StopAllButton({ compact = false, onStopped }: ButtonProps) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className={compact ? 'hq-btn-stop !h-11 !w-11 !p-0' : 'hq-btn-stop'}
        aria-label="Stop every agent"
        title="Stop every agent"
      >
        <OctagonX className="w-4 h-4" />
        {!compact && 'Stop all'}
      </button>
      {open && <StopAllDialog onClose={() => setOpen(false)} onStopped={onStopped} />}
    </>
  )
}

function StopAllDialog({ onClose, onStopped }: { onClose: () => void; onStopped: () => void }) {
  const [hosts, setHosts] = useState<WorkbenchHost[]>([])
  const [target, setTarget] = useState(ALL_COMPUTERS)
  const [reason, setReason] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<LockdownResult | null>(null)
  const close = () => {
    if (!busy) onClose()
  }
  const { dialogRef, portal, trapTab } = useModal(close)

  // The stop never waits for this list: every computer is the default.
  useEffect(() => {
    workbenchApi
      .hosts()
      .then((res) => setHosts(res.hosts))
      .catch(() => setHosts([]))
  }, [])

  const stop = async () => {
    setBusy(true)
    setError(null)
    try {
      const done = await workbenchApi.lockdown({
        ...(reason.trim() ? { reason: reason.trim() } : {}),
        ...(target ? { host: target } : {}),
      })
      setResult(done)
      onStopped()
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not stop the agents.')
    } finally {
      setBusy(false)
    }
  }

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center hq-scrim sm:p-4">
      <div
        ref={dialogRef}
        tabIndex={-1}
        onKeyDown={trapTab}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="stop-all-title"
        aria-describedby="stop-all-what"
        className="w-full max-w-md max-h-[92dvh] outline-none flex flex-col rounded-t-2xl sm:rounded-2xl hq-modal overflow-hidden"
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
          <h2 id="stop-all-title" className="text-sm font-bold text-white">
            Stop every agent
          </h2>
          <button type="button" onClick={close} disabled={busy} aria-label="Close" className="disabled:opacity-40 flex items-center justify-center h-11 w-11 -mr-2 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10">
            <X className="w-4 h-4" />
          </button>
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto overscroll-contain px-4 py-4 space-y-4">
          {result ? (
            <StopReport result={result} />
          ) : (
            <>
              <p id="stop-all-what" className="text-xs text-neutral-300 leading-relaxed">
                Every agent on the computer is killed at once, and it starts none until someone at that computer runs{' '}
                <code className="text-neutral-100">hq host unlock</code>. Stopped agents do not come back by themselves.
              </p>
              <div>
                <label htmlFor="stop-all-computer" className="block mb-1 text-[11px] text-neutral-400">
                  Computer
                </label>
                <select
                  id="stop-all-computer"
                  value={target}
                  onChange={(e) => setTarget(e.target.value)}
                  className="w-full min-w-0 h-11 px-3 rounded-lg text-xs text-neutral-200 hq-field"
                >
                  <option value={ALL_COMPUTERS}>Every computer</option>
                  {hosts.map((h) => (
                    <option key={h.host} value={h.host}>
                      {computerName(h.host)}
                    </option>
                  ))}
                </select>
              </div>
              <div>
                <label htmlFor="stop-all-reason" className="block mb-1 text-[11px] text-neutral-400">
                  Why (optional)
                </label>
                <input
                  id="stop-all-reason"
                  value={reason}
                  maxLength={MAX_REASON_CHARS}
                  onChange={(e) => setReason(e.target.value)}
                  placeholder="Agents I did not start"
                  className="w-full min-w-0 h-11 px-3 rounded-lg text-xs text-neutral-200 hq-field"
                />
              </div>
              {error && <ErrorText>{error}</ErrorText>}
            </>
          )}
        </div>
        <div className="px-4 py-3 border-t border-white/10 pb-[calc(0.75rem+var(--safe-bottom))]">
          {result ? (
            <button type="button" onClick={close} className="hq-btn-primary w-full">
              Done
            </button>
          ) : (
            <button type="button" onClick={() => void stop()} disabled={busy} className="hq-btn-primary w-full">
              {busy ? <Loader2 className="w-4 h-4 animate-spin" /> : <OctagonX className="w-4 h-4" />}
              {busy ? 'Stopping' : target ? `Stop every agent on ${computerName(target)}` : 'Stop every agent everywhere'}
            </button>
          )}
        </div>
      </div>
    </div>,
    portal
  )
}

function StopReport({ result }: { result: LockdownResult }) {
  return (
    <ul className="space-y-2" role="status">
      {result.hosts.map((h) => (
        <li key={h.host} className="flex items-start gap-2 text-xs">
          {h.locked ? (
            <CheckCircle2 className="w-4 h-4 shrink-0" style={{ color: 'var(--accent-green)' }} />
          ) : (
            <OctagonX className="w-4 h-4 shrink-0" style={{ color: 'var(--accent-amber)' }} />
          )}
          <span className="min-w-0 break-words text-neutral-200">
            {h.locked
              ? `${computerName(h.host)} is stopped and locked.`
              : `${computerName(h.host)} did not lock: it may be offline or need an HQ update. Run hq host lockdown on it.`}
          </span>
        </li>
      ))}
    </ul>
  )
}

/** Names each computer that is locked down and how to lift it. */
export function LockdownBanner({ hosts }: { hosts: WorkbenchHost[] }) {
  const locked = hosts.filter((h) => h.lockdown)
  if (locked.length === 0) return null
  return (
    <div role="status" className="mx-4 mb-3 px-3 py-2 rounded-xl text-xs text-neutral-200 border" style={{ borderColor: 'var(--accent-green)' }}>
      {locked.map((h) => (
        <p key={h.host} className="break-words">
          <span className="font-semibold" style={{ color: 'var(--accent-green)' }}>
            {computerName(h.host)} is locked down
          </span>{' '}
          ({h.lockdown?.reason}). Run <code>hq host unlock</code> on it to start agents again.
        </p>
      ))}
    </div>
  )
}
