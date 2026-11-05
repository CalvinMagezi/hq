import { t } from '~/strings'
import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { Loader2, X } from 'lucide-react'
import { GENERAL_PURPOSES, workbenchApi, type LaunchReport, type Purpose, type WorkbenchHosts } from '~/lib/sessionsApi'
import {
  DEFAULT_AGENT,
  agentCategory,
  agentName,
  computerName,
  computerUnavailableReason,
  harnessesIn,
  plainPath,
} from '~/lib/workbench'
import { ErrorText } from './ErrorText'
import { FolderPicker } from './FolderPicker'
import { LockdownBanner } from './EmergencyStop'
import { useModal } from './useModal'

const FIELD_CLASS =
  'w-full min-w-0 px-3 rounded-lg text-xs text-neutral-200 hq-field focus:outline-none focus:ring-1 focus:ring-emerald-400'
const PURPOSE_WORDS: Record<Purpose, 'helpers_new_purpose_general' | 'helpers_new_purpose_research' | 'helpers_new_purpose_review' | 'helpers_new_purpose_monitor'> = {
  code: 'helpers_new_purpose_general',
  general: 'helpers_new_purpose_general',
  research: 'helpers_new_purpose_research',
  review: 'helpers_new_purpose_review',
  monitor: 'helpers_new_purpose_monitor',
}
const LABEL_CLASS = 'block mb-1 text-[11px] text-neutral-400'

interface Props {
  onClose: () => void
  /** Called with the new agent's id once it has started. */
  onStarted: (sessionId: string) => void
}

const PHONE_QUERY = '(max-width: 639px)'

function useIsPhone(): boolean {
  const [phone, setPhone] = useState(false)
  useEffect(() => {
    const q = window.matchMedia(PHONE_QUERY)
    const sync = () => setPhone(q.matches)
    sync()
    q.addEventListener('change', sync)
    return () => q.removeEventListener('change', sync)
  }, [])
  return phone
}

/** Pick a computer, a folder and an agent, say what to do first, and start. */
export function NewAgentDialog({ onClose, onStarted }: Props) {
  const [hosts, setHosts] = useState<WorkbenchHosts | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [host, setHost] = useState('')
  const [folder, setFolder] = useState('')
  const [harness, setHarness] = useState('')
  const [purpose, setPurpose] = useState<Purpose>('general')
  const [prompt, setPrompt] = useState('')
  const [label, setLabel] = useState('')
  const [branch, setBranch] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [report, setReport] = useState<LaunchReport | null>(null)
  const phone = useIsPhone()
  const [foldOpen, setFoldOpen] = useState(false)
  const [changingFolder, setChangingFolder] = useState(false)

  useEffect(() => {
    workbenchApi
      .hosts()
      .then((res) => {
        setHosts(res)
        setHost(res.hosts.find((h) => !computerUnavailableReason(h))?.host ?? '')
        setHarness(res.harnesses.includes(DEFAULT_AGENT) ? DEFAULT_AGENT : (res.harnesses[0] ?? ''))
      })
      .catch((e) => setLoadError(e instanceof Error ? e.message : t('helpers_load_failed')))
  }, [])

  // Once an agent exists, closing means going to it.
  const close = () => {
    if (busy) return
    if (report) onStarted(report.session_id)
    else onClose()
  }
  const { dialogRef, portal, trapTab } = useModal(close)
  const selected = hosts?.hosts.find((h) => h.host === host)
  const noneReachable = Boolean(hosts && !hosts.hosts.some((h) => h.reachable))
  // A general-purpose agent is started for a stated purpose; a coding agent's purpose is code.
  const generalPurpose = agentCategory(hosts, harness) === 'general_purpose'
  const start = async () => {
    setBusy(true)
    setError(null)
    try {
      const launched = await workbenchApi.spawn({
        harness,
        host,
        ...(generalPurpose ? { purpose } : {}),
        ...(folder ? { folder } : {}),
        ...(prompt.trim() ? { prompt: prompt.trim() } : {}),
        ...(label.trim() ? { label: label.trim() } : {}),
        ...(!generalPurpose && branch.trim() ? { branch: branch.trim() } : {}),
      })
      if (launched.blocked || launched.note) setReport(launched)
      else onStarted(launched.session_id)
    } catch (e) {
      setError(`${t('helpers_new_failed', { helper: agentName(harness) })} ${e instanceof Error ? e.message : ''}`.trim())
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
            {t('helpers_start_a_helper')}
          </h2>
          <button type="button" onClick={close} disabled={busy} aria-label={t('helpers_new_cancel')} className="disabled:opacity-40 flex items-center justify-center h-11 w-11 -mr-2 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10">
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="flex-1 min-h-0 overflow-y-auto overscroll-contain px-4 py-4 space-y-4">
          {loadError && <ErrorText>{loadError}</ErrorText>}
          {report && <LaunchNotice report={report} />}
          {!hosts && !loadError && (
            <div role="status" className="flex items-center gap-2 text-xs text-neutral-500">
              <Loader2 className="w-4 h-4 animate-spin" />
              {t('helpers_new_loading')}
            </div>
          )}
          {hosts && !report && (
            <>
              {hosts.hosts.some((h) => h.lockdown) && <LockdownBanner hosts={hosts.hosts} />}
              <div>
                <label htmlFor="agent-first" className={LABEL_CLASS}>
                  {t('helpers_new_first')}
                </label>
                <textarea id="agent-first" value={prompt} onChange={(e) => setPrompt(e.target.value)} rows={3} className={`${FIELD_CLASS} py-2 resize-y`} />
              </div>

              <div>
                <label htmlFor="agent-kind" className={LABEL_CLASS}>
                  {t('helpers_new_which')}
                </label>
                <select id="agent-kind" value={harness} onChange={(e) => setHarness(e.target.value)} className={`${FIELD_CLASS} h-11 sm:h-9`}>
                  {(['coding', 'general_purpose'] as const)
                    .map((category) => [category, harnessesIn(hosts, category)] as const)
                    .filter(([, names]) => names.length > 0)
                    .map(([category, names]) => (
                      <optgroup key={category} label={t(category === 'coding' ? 'helpers_new_group_coding' : 'helpers_new_group_general')}>
                        {names.map((h) => (
                          <option key={h} value={h}>
                            {agentName(h)}
                          </option>
                        ))}
                      </optgroup>
                    ))}
                </select>
              </div>

              {generalPurpose && (
                <div>
                  <label htmlFor="agent-purpose" className={LABEL_CLASS}>
                    {t('helpers_new_purpose')}
                  </label>
                  <select id="agent-purpose" value={purpose} onChange={(e) => setPurpose(e.target.value as Purpose)} className={`${FIELD_CLASS} h-11 sm:h-9`}>
                    {GENERAL_PURPOSES.map((p) => (
                      <option key={p} value={p}>
                        {t(PURPOSE_WORDS[p])}
                      </option>
                    ))}
                  </select>
                  <p className="mt-1 text-[11px] text-neutral-500">{t('helpers_new_purpose_hint')}</p>
                </div>
              )}

              {!phone && (
                <div>
                  <label htmlFor="agent-name" className={LABEL_CLASS}>
                    {t('helpers_new_name')}
                  </label>
                  <input id="agent-name" value={label} onChange={(e) => setLabel(e.target.value)} maxLength={80} className={`${FIELD_CLASS} h-11 sm:h-9`} />
                </div>
              )}

              <div>
                <label htmlFor="agent-computer" className={LABEL_CLASS}>
                  {t('helpers_new_where')}
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
                  {hosts.hosts.map((h) => (
                    <option key={h.host} value={h.host} disabled={computerUnavailableReason(h) !== null}>
                      {h.reachable ? computerName(h.host) : t('helpers_new_computer_down', { computer: computerName(h.host) })}
                    </option>
                  ))}
                </select>
                {noneReachable && (
                  <p role="status" className="mt-1 text-[11px]" style={{ color: 'var(--warn)' }}>
                    {t('helpers_new_none_reachable', { computer: computerName(hosts.hosts[0]?.host ?? '') })} {t('helpers_new_needs_computer')}
                  </p>
                )}
              </div>

              {selected?.workspace && (
                <div>
                  <span className={LABEL_CLASS}>{t('helpers_new_folder')}</span>
                  {changingFolder ? (
                    <FolderPicker host={host} workspace={selected.workspace} value={folder} onChange={setFolder} />
                  ) : (
                    <div className="flex items-center justify-between gap-2">
                      <span className="min-w-0 break-all text-xs text-neutral-300">{plainPath(folder || selected.workspace.root)}</span>
                      <button type="button" onClick={() => setChangingFolder(true)} className="a-btn secondary shrink-0">
                        {t('helpers_new_change')}
                      </button>
                    </div>
                  )}
                </div>
              )}

              {phone ? (
                <details open={foldOpen || label !== '' || branch !== ''} onToggle={(e) => setFoldOpen(e.currentTarget.open)}>
                  <summary className="flex items-center min-h-11 cursor-pointer text-[11px] text-neutral-400 select-none">{t('helpers_new_name_advanced')}</summary>
                  <div className="space-y-4">
                    <NameField label={label} onChange={setLabel} />
                    {!generalPurpose && <BranchField branch={branch} onChange={setBranch} />}
                  </div>
                </details>
              ) : (
                !generalPurpose && (
                  <details>
                    <summary className="flex items-center min-h-11 cursor-pointer text-[11px] text-neutral-400 select-none">{t('helpers_new_advanced')}</summary>
                    <BranchField branch={branch} onChange={setBranch} />
                  </details>
                )
              )}
            </>
          )}
        </div>

        <div className="px-4 py-3 border-t border-white/10 space-y-2">
          {error && <ErrorText>{error}</ErrorText>}
          {selected?.lockdown && !report && <p role="status" className="text-[11px] text-neutral-400">{t('helpers_new_locked')}</p>}
          {report ? (
            <button type="button" onClick={() => onStarted(report.session_id)} className={`${PRIMARY_CLASS} disabled:opacity-40`}>
              {t('helpers_new_open')}
            </button>
          ) : (
          <div className="flex gap-2">
          <button type="button" onClick={close} disabled={busy} className="a-btn secondary flex-1">
            {t('helpers_new_cancel')}
          </button>
          <button
            type="button"
            onClick={() => void start()}
            disabled={busy || !selected?.workspace || !harness || Boolean(selected && computerUnavailableReason(selected)) || Boolean(selected?.lockdown)}
            className={`${PRIMARY_CLASS} flex-1 disabled:opacity-40 disabled:hover:bg-white/10`}
          >
            {busy && <Loader2 className="w-4 h-4 animate-spin" />}
            {busy ? t('helpers_new_starting') : t('helpers_new_start')}
          </button>
          </div>
          )}
          <p className="text-[11px] text-neutral-500">{t('helpers_new_asks')}</p>
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
      <p className="text-sm font-semibold text-white">{t('helpers_new_started', { helper: agentName(report.harness) })}</p>
      {report.note && <p className="text-xs text-neutral-300 break-words">{report.note}</p>}
      {report.blocked && (
        <>
          <p className="text-xs " style={{ color: 'var(--accent-amber)' }}>
            {t('helpers_new_blocked')}
          </p>
          <pre className="max-h-48 overflow-auto rounded-lg bg-black/40 px-3 py-2 text-[11px] leading-snug text-neutral-200 whitespace-pre-wrap break-words">{report.blocked.screen}</pre>
        </>
      )}
    </div>
  )
}

function NameField({ label, onChange }: { label: string; onChange: (v: string) => void }) {
  return (
    <div>
      <label htmlFor="agent-name" className={LABEL_CLASS}>
        {t('helpers_new_name')}
      </label>
      <input id="agent-name" value={label} onChange={(e) => onChange(e.target.value)} maxLength={80} className={`${FIELD_CLASS} h-11 sm:h-9`} />
    </div>
  )
}

function BranchField({ branch, onChange }: { branch: string; onChange: (v: string) => void }) {
  return (
    <div>
      <label htmlFor="agent-branch" className={LABEL_CLASS}>
        {t('helpers_new_branch')}
      </label>
      <input id="agent-branch" value={branch} onChange={(e) => onChange(e.target.value)} maxLength={120} className={`${FIELD_CLASS} h-11 sm:h-9`} />
      <p className="mt-1 text-[11px] text-neutral-500">{t('helpers_new_branch_hint')}</p>
    </div>
  )
}
