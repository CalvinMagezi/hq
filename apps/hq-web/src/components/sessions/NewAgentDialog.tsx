import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Loader2, X } from 'lucide-react'
import { workbenchApi, type LaunchReport, type WorkbenchHosts } from '~/lib/sessionsApi'
import { DEFAULT_AGENT, agentName, computerName, computerUnavailableReason } from '~/lib/workbench'
import { ErrorText } from './ErrorText'
import { FolderPicker } from './FolderPicker'

const FIELD_CLASS =
  'w-full min-w-0 px-3 rounded-lg text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400'
const LABEL_CLASS = 'block mb-1 text-[11px] text-neutral-400'

interface Props {
  onClose: () => void
  /** Called with the new agent's id once it has started. */
  onStarted: (sessionId: string) => void
}

/** Pick a computer, a folder and an agent, say what to do first, and start. */
export function NewAgentDialog({ onClose, onStarted }: Props) {
  const [hosts, setHosts] = useState<WorkbenchHosts | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [host, setHost] = useState('')
  const [folder, setFolder] = useState('')
  const [harness, setHarness] = useState('')
  const [prompt, setPrompt] = useState('')
  const [label, setLabel] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [report, setReport] = useState<LaunchReport | null>(null)
  const dialogRef = useRef<HTMLDivElement>(null)
  const [portal] = useState(() => document.createElement('div'))

  // Mount in a portal, hide the rest of the page from assistive tech and the keyboard, and hand focus back afterwards.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null
    document.body.appendChild(portal)
    const hidden = Array.from(document.body.children).filter((el) => el !== portal && !el.hasAttribute('inert'))
    hidden.forEach((el) => el.setAttribute('inert', ''))
    dialogRef.current?.focus()
    return () => {
      hidden.forEach((el) => el.removeAttribute('inert'))
      portal.remove()
      opener?.focus()
    }
  }, [portal])

  useEffect(() => {
    workbenchApi
      .hosts()
      .then((res) => {
        setHosts(res)
        setHost(res.hosts.find((h) => !computerUnavailableReason(h))?.host ?? '')
        setHarness(res.harnesses.includes(DEFAULT_AGENT) ? DEFAULT_AGENT : (res.harnesses[0] ?? ''))
      })
      .catch((e) => setLoadError(e instanceof Error ? e.message : 'Could not load your computers.'))
  }, [])

  // Once an agent exists, closing means going to it.
  const close = () => {
    if (busy) return
    if (report) onStarted(report.session_id)
    else onClose()
  }
  const closeRef = useRef(close)
  closeRef.current = close
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && closeRef.current()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const trapTab = (e: React.KeyboardEvent) => {
    if (e.key !== 'Tab') return
    const items = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>('button, input, select, textarea, summary, [href], [tabindex]:not([tabindex="-1"])') ?? []).filter(
      (el) => !el.hasAttribute('disabled') && el.offsetParent !== null
    )
    if (items.length === 0) return
    const first = items[0]
    const last = items[items.length - 1]
    const active = document.activeElement
    if (e.shiftKey && (active === first || active === dialogRef.current)) {
      e.preventDefault()
      last.focus()
    } else if (!e.shiftKey && active === last) {
      e.preventDefault()
      first.focus()
    }
  }

  const selected = hosts?.hosts.find((h) => h.host === host)
  const start = async () => {
    setBusy(true)
    setError(null)
    try {
      const launched = await workbenchApi.spawn({
        harness,
        host,
        ...(folder ? { folder } : {}),
        ...(prompt.trim() ? { prompt: prompt.trim() } : {}),
        ...(label.trim() ? { label: label.trim() } : {}),
      })
      if (launched.blocked || launched.note) setReport(launched)
      else onStarted(launched.session_id)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not start the agent.')
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
        role="dialog"
        aria-modal="true"
        aria-labelledby="new-agent-title"
        className="w-full max-w-lg max-h-[92dvh] outline-none flex flex-col rounded-t-2xl sm:rounded-2xl hq-modal overflow-hidden"
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
          <h2 id="new-agent-title" className="text-sm font-bold text-white">
            New agent
          </h2>
          <button type="button" onClick={close} disabled={busy} aria-label="Close" className="disabled:opacity-40 flex items-center justify-center h-11 w-11 -mr-2 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10">
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="flex-1 min-h-0 overflow-y-auto overscroll-contain px-4 py-4 space-y-4">
          {loadError && <ErrorText>{loadError}</ErrorText>}
          {report && <LaunchNotice report={report} />}
          {!hosts && !loadError && (
            <div role="status" className="flex items-center gap-2 text-xs text-neutral-500">
              <Loader2 className="w-4 h-4 animate-spin" />
              Looking for your computers
            </div>
          )}
          {hosts && !report && (
            <>
              <div>
                <label htmlFor="agent-computer" className={LABEL_CLASS}>
                  Computer
                </label>
                <select
                  id="agent-computer"
                  value={host}
                  onChange={(e) => {
                    setHost(e.target.value)
                    setFolder('')
                  }}
                  className={`${FIELD_CLASS} h-11 sm:h-9`}
                >
                  {hosts.hosts.map((h) => {
                    const reason = computerUnavailableReason(h)
                    return (
                      <option key={h.host} value={h.host} disabled={reason !== null}>
                        {computerName(h.host)}
                        {reason ? ' (unavailable)' : ''}
                      </option>
                    )
                  })}
                </select>
                {hosts.hosts.filter((h) => computerUnavailableReason(h)).map((h) => (
                  <p key={h.host} className="mt-1 text-[11px] " style={{ color: 'var(--accent-amber)' }}>
                    {computerUnavailableReason(h)}
                  </p>
                ))}
              </div>

              {selected?.workspace && (
                <div>
                  <span className={LABEL_CLASS}>Folder</span>
                  <FolderPicker host={host} workspace={selected.workspace} value={folder} onChange={setFolder} />
                </div>
              )}

              <div>
                <label htmlFor="agent-kind" className={LABEL_CLASS}>
                  Agent
                </label>
                <select id="agent-kind" value={harness} onChange={(e) => setHarness(e.target.value)} className={`${FIELD_CLASS} h-11 sm:h-9`}>
                  {hosts.harnesses.map((h) => (
                    <option key={h} value={h}>
                      {agentName(h)}
                    </option>
                  ))}
                </select>
              </div>

              <div>
                <label htmlFor="agent-first" className={LABEL_CLASS}>
                  What should it do first? (optional)
                </label>
                <textarea id="agent-first" value={prompt} onChange={(e) => setPrompt(e.target.value)} rows={3} className={`${FIELD_CLASS} py-2 resize-y`} />
              </div>

              <div>
                <label htmlFor="agent-name" className={LABEL_CLASS}>
                  Name (optional)
                </label>
                <input id="agent-name" value={label} onChange={(e) => setLabel(e.target.value)} maxLength={80} className={`${FIELD_CLASS} h-11 sm:h-9`} />
              </div>
            </>
          )}
        </div>

        <div className="px-4 py-3 border-t border-white/10 space-y-2">
          {error && <ErrorText>{error}</ErrorText>}
          {report ? (
            <button type="button" onClick={() => onStarted(report.session_id)} className={`${PRIMARY_CLASS} disabled:opacity-40`}>
              Open the agent
            </button>
          ) : (
          <button
            type="button"
            onClick={() => void start()}
            disabled={busy || !selected?.workspace || !harness || Boolean(selected && computerUnavailableReason(selected))}
            className={`${PRIMARY_CLASS} disabled:opacity-40 disabled:hover:bg-white/10`}
          >
            {busy && <Loader2 className="w-4 h-4 animate-spin" />}
            Start agent
          </button>
          )}
          <p className="text-[11px] text-neutral-500">The agent asks before it runs commands or changes files. You approve each step here.</p>
        </div>
      </div>
    </div>,
    portal
  )
}

const PRIMARY_CLASS = 'w-full flex items-center justify-center gap-2 h-11 rounded-xl border border-white/10 bg-white/10 text-sm font-semibold text-white hover:bg-white/15'

/** What the agent said right after starting, shown before the dialog closes so a question is never missed. */
function LaunchNotice({ report }: { report: LaunchReport }) {
  return (
    <div className="space-y-2" role="status">
      <p className="text-sm font-semibold text-white">The agent has started.</p>
      {report.note && <p className="text-xs text-neutral-300 break-words">{report.note}</p>}
      {report.blocked && (
        <>
          <p className="text-xs " style={{ color: 'var(--accent-amber)' }}>
            It is already waiting for you. You can answer once it opens.
          </p>
          <pre className="max-h-48 overflow-auto rounded-lg bg-black/40 px-3 py-2 text-[11px] leading-snug text-neutral-200 whitespace-pre-wrap break-words">{report.blocked.screen}</pre>
        </>
      )}
    </div>
  )
}
