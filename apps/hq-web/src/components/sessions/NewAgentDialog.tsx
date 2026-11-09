import { t } from '~/strings'
import { useEffect, useState } from 'react'
import { Loader2 } from 'lucide-react'
import { GENERAL_PURPOSES, attachSearch, workbenchApi, type AttachHit, type LaunchReport, type Purpose, type WorkbenchHosts } from '~/lib/sessionsApi'
import {
  agentCategory,
  agentName,
  agentsOn,
  computerName,
  computerUnavailableReason,
  fitAgent,
  harnessesIn,
  plainPath,
} from '~/lib/workbench'
import { AttachSearch } from './AttachSearch'
import { ErrorText } from './ErrorText'
import { FolderPicker } from './FolderPicker'
import { LockdownBanner } from './EmergencyStop'
import { Sheet } from '../ui/Sheet'

const FIELD_CLASS = 'a-input'
const PURPOSE_WORDS: Record<Purpose, 'helpers_new_purpose_general' | 'helpers_new_purpose_research' | 'helpers_new_purpose_review' | 'helpers_new_purpose_monitor'> = {
  code: 'helpers_new_purpose_general',
  general: 'helpers_new_purpose_general',
  research: 'helpers_new_purpose_research',
  review: 'helpers_new_purpose_review',
  monitor: 'helpers_new_purpose_monitor',
}
const LABEL_CLASS = 'a-field-label'
const NOTHING_ATTACHED = null as AttachHit | null

interface Props {
  onClose: () => void
  /** Called with the new agent's id once it has started. */
  onStarted: (sessionId: string) => void
  /** Start it on this task (WR17): the task records its work. */
  task?: { id: string; title: string }
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
export function NewAgentDialog({ onClose, onStarted, task }: Props) {
  const [hosts, setHosts] = useState<WorkbenchHosts | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [host, setHost] = useState('')
  const [folder, setFolder] = useState('')
  const [harness, setHarness] = useState('')
  const [purpose, setPurpose] = useState<Purpose>('general')
  const [prompt, setPrompt] = useState('')
  const [label, setLabel] = useState('')
  const [branch, setBranch] = useState('')
  const [note, setNote] = useState(NOTHING_ATTACHED)
  const [attachedTask, setAttachedTask] = useState(NOTHING_ATTACHED)
  const [repo, setRepo] = useState('')
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
      })
      .catch((e) => setLoadError(e instanceof Error ? e.message : t('helpers_load_failed')))
  }, [])

  // Once an agent exists, closing means going to it.
  const close = () => {
    if (busy) return
    if (report) onStarted(report.session_id)
    else onClose()
  }
  const selected = hosts?.hosts.find((h) => h.host === host)
  const noneReachable = Boolean(hosts && !hosts.hosts.some((h) => h.reachable))
  // A computer can have fewer agents than the last one, so the choice is checked when it changes.
  useEffect(() => {
    if (hosts) setHarness((current) => fitAgent(current, agentsOn(selected, hosts.harnesses)))
  }, [hosts, selected])
  const taskId = task?.id ?? attachedTask?.id
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
        ...(taskId ? { task_id: taskId } : {}),
        ...(note ? { note: note.id } : {}),
        ...(repo.trim() ? { repo: repo.trim() } : {}),
      })
      if (launched.blocked || launched.note) setReport(launched)
      else onStarted(launched.session_id)
    } catch (e) {
      setError(`${t('helpers_new_failed', { helper: agentName(harness) })} ${e instanceof Error ? e.message : ''}`.trim())
    } finally {
      setBusy(false)
    }
  }

  const footer = report ? (
    <button type="button" onClick={() => onStarted(report.session_id)} className="a-btn">
      {t('helpers_new_open')}
    </button>
  ) : (
    <>
      <button type="button" onClick={close} disabled={busy} className="a-btn secondary">
        {t('helpers_new_cancel')}
      </button>
      <button
        type="button"
        onClick={() => void start()}
        disabled={busy || !selected?.workspace || !harness || Boolean(selected && computerUnavailableReason(selected)) || Boolean(selected?.lockdown)}
        className="a-btn"
      >
        {busy && <Loader2 className="a-ic animate-spin" aria-hidden="true" />}
        {busy ? t('helpers_new_starting') : t('helpers_new_start')}
      </button>
    </>
  )

  return (
    <Sheet title={t('helpers_start_a_helper')} onClose={close} busy={busy} footer={footer}>
      <div className="flex flex-col gap-4">
          {loadError && <ErrorText>{loadError}</ErrorText>}
          {report && <LaunchNotice report={report} />}
          {!hosts && !loadError && (
            <div role="status" className="flex items-center gap-2 a-body-sm a-muted">
              <Loader2 className="a-ic animate-spin" aria-hidden="true" />
              {t('helpers_new_loading')}
            </div>
          )}
          {hosts && !report && (
            <>
              {hosts.hosts.some((h) => h.lockdown) && <LockdownBanner hosts={hosts.hosts} />}
              {task && <p className="a-body-sm">{t('helpers_new_for_task', { title: task.title })}</p>}
              <div className="a-field">
                <label htmlFor="agent-first" className={LABEL_CLASS}>
                  {t('helpers_new_first')}
                </label>
                <textarea id="agent-first" value={prompt} onChange={(e) => setPrompt(e.target.value)} rows={3} className={`${FIELD_CLASS} resize-y`} />
              </div>

              <div className="a-field">
                <label htmlFor="agent-kind" className={LABEL_CLASS}>
                  {t('helpers_new_which')}
                </label>
                <select id="agent-kind" value={harness} onChange={(e) => setHarness(e.target.value)} className={FIELD_CLASS}>
                  {(['coding', 'general_purpose'] as const)
                    .map((category) => [category, agentsOn(selected, harnessesIn(hosts, category))] as const)
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

              {selected?.agents?.length === 0 && <p role="status" className="a-cap a-warn-t">{t('helpers_agents_none')}</p>}

              <AttachSearch id="agent-note" label={t('helpers_attach_note')} placeholder={t('helpers_attach_search_notes')} value={note} onChange={setNote} search={attachSearch.notes} />
              {!task && <AttachSearch id="agent-task" label={t('helpers_attach_task')} placeholder={t('helpers_attach_search_tasks')} value={attachedTask} onChange={setAttachedTask} search={attachSearch.tasks} />}
              <div className="a-field">
                <label htmlFor="agent-repo" className={LABEL_CLASS}>
                  {t('helpers_attach_repo')}
                </label>
                <input id="agent-repo" value={repo} onChange={(e) => setRepo(e.target.value)} placeholder={t('helpers_attach_repo_hint')} className={FIELD_CLASS} />
              </div>

              {generalPurpose && (
                <div className="a-field">
                  <span id="agent-purpose-label" className={LABEL_CLASS}>
                    {t('helpers_new_purpose')}
                  </span>
                  <div id="agent-purpose" role="group" aria-labelledby="agent-purpose-label" className="a-chips">
                    {GENERAL_PURPOSES.map((p) => (
                      <button key={p} type="button" aria-pressed={purpose === p} onClick={() => setPurpose(p)} className={`a-chip ${purpose === p ? 'on' : ''}`}>
                        {t(PURPOSE_WORDS[p])}
                      </button>
                    ))}
                  </div>
                  <p className="a-cap a-muted">{t('helpers_new_purpose_hint')}</p>
                </div>
              )}

              {!phone && (
                <div className="a-field">
                  <label htmlFor="agent-name" className={LABEL_CLASS}>
                    {t('helpers_new_name')}
                  </label>
                  <input id="agent-name" value={label} onChange={(e) => setLabel(e.target.value)} maxLength={80} className={FIELD_CLASS} />
                </div>
              )}

              <div className="a-field">
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
                  className={FIELD_CLASS}
                >
                  {hosts.hosts.map((h) => (
                    <option key={h.host} value={h.host} disabled={computerUnavailableReason(h) !== null}>
                      {h.reachable ? computerName(h.host) : t('helpers_new_computer_down', { computer: computerName(h.host) })}
                    </option>
                  ))}
                </select>
                {noneReachable && (
                  <p role="status" className="a-cap a-warn-t">
                    {t('helpers_new_none_reachable', { computer: computerName(hosts.hosts[0]?.host ?? '') })} {t('helpers_new_needs_computer')}
                  </p>
                )}
              </div>

              {selected?.workspace && (
                <div className="a-field">
                  <span className={LABEL_CLASS}>{t('helpers_new_folder')}</span>
                  {changingFolder ? (
                    <FolderPicker host={host} workspace={selected.workspace} value={folder} onChange={setFolder} />
                  ) : (
                    <div className="flex items-center justify-between gap-2">
                      <span className="min-w-0 break-all a-body-sm">{plainPath(folder || selected.workspace.root)}</span>
                      <button type="button" onClick={() => setChangingFolder(true)} className="a-btn secondary shrink-0">
                        {t('helpers_new_change')}
                      </button>
                    </div>
                  )}
                </div>
              )}

              {phone ? (
                <details open={foldOpen || label !== '' || branch !== ''} onToggle={(e) => setFoldOpen(e.currentTarget.open)}>
                  <summary className="flex items-center min-h-11 cursor-pointer a-body-sm a-muted select-none">{t('helpers_new_name_advanced')}</summary>
                  <div className="flex flex-col gap-4">
                    <NameField label={label} onChange={setLabel} />
                    {!generalPurpose && <BranchField branch={branch} onChange={setBranch} />}
                  </div>
                </details>
              ) : (
                !generalPurpose && (
                  <details>
                    <summary className="flex items-center min-h-11 cursor-pointer a-body-sm a-muted select-none">{t('helpers_new_advanced')}</summary>
                    <BranchField branch={branch} onChange={setBranch} />
                  </details>
                )
              )}
            </>
          )}
        {error && <ErrorText>{error}</ErrorText>}
        {selected?.lockdown && !report && <p role="status" className="a-cap a-muted">{t('helpers_new_locked')}</p>}
        <p className="a-cap a-muted">{t('helpers_new_asks')}</p>
      </div>
    </Sheet>
  )
}

/** What the agent said right after starting, shown before the dialog closes so a question is never missed. */
function LaunchNotice({ report }: { report: LaunchReport }) {
  return (
    <div className="flex flex-col gap-2" role="status">
      <p className="a-body-b">{t('helpers_new_started', { helper: agentName(report.harness) })}</p>
      {report.note && <p className="a-body-sm break-words">{report.note}</p>}
      {report.blocked && (
        <>
          <p className="a-body-sm a-warn-t">{t('helpers_new_blocked')}</p>
          <pre className="a-quote max-h-48 overflow-auto break-words">{report.blocked.screen}</pre>
        </>
      )}
    </div>
  )
}

function NameField({ label, onChange }: { label: string; onChange: (v: string) => void }) {
  return (
    <div className="a-field">
      <label htmlFor="agent-name" className={LABEL_CLASS}>
        {t('helpers_new_name')}
      </label>
      <input id="agent-name" value={label} onChange={(e) => onChange(e.target.value)} maxLength={80} className={FIELD_CLASS} />
    </div>
  )
}

function BranchField({ branch, onChange }: { branch: string; onChange: (v: string) => void }) {
  return (
    <div className="a-field">
      <label htmlFor="agent-branch" className={LABEL_CLASS}>
        {t('helpers_new_branch')}
      </label>
      <input id="agent-branch" value={branch} onChange={(e) => onChange(e.target.value)} maxLength={120} className={FIELD_CLASS} />
      <p className="a-cap a-muted">{t('helpers_new_branch_hint')}</p>
    </div>
  )
}
