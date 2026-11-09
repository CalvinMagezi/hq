import { useCallback, useEffect, useState } from 'react'
import { ErrorText } from '~/components/sessions/ErrorText'
import { edgeWorkerApi } from '~/lib/edgeWorker'
import { githubApi, githubTask, type GithubLink, type GithubRepo, type GithubRun } from '~/lib/github'
import { createTaskClient } from '~/lib/tasksApi'
import { t } from '~/strings'

const BUDGET_DECIMALS = 2

const open = (url: string | null | undefined) => {
  if (url) window.open(url, '_blank', 'noopener')
}

function Repo({ repo, onAllow }: { repo: GithubRepo; onAllow: () => void }) {
  const { report } = repo
  return (
    <li className="a-card">
      <b className="a-body-b">{report.repo}</b>
      <p className="a-body-sm">{t(report.has_ci ? 'github_has_ci' : 'github_no_ci')}</p>
      <p className="a-body-sm">{t(report.protected ? 'github_protected' : 'github_unprotected')}</p>
      {report.secrets.length > 0 && <p className="a-body-sm a-warn-t">{t('github_uses_secrets', { names: report.secrets.join(', ') })}</p>}
      {report.previews.length > 0 && <p className="a-body-sm a-warn-t">{t('github_uses_previews', { names: report.previews.join(', ') })}</p>}
      {repo.flagged && !repo.picked && (
        <>
          <p className="a-body-sm">{t('github_flag_why')}</p>
          <button type="button" className="a-btn secondary compact" onClick={onAllow}>
            {t('github_allow_repo')}
          </button>
        </>
      )}
    </li>
  )
}

/** Giving HQ its work next to the connection. Pressing Start, with the budget in view, is the first-task yes. */
function JobForm({ link, run }: { link: GithubLink; run: GithubRun }) {
  const picked = link.repos.filter((r) => r.picked).map((r) => r.report.repo)
  const [chosen, setChosen] = useState('')
  const [job, setJob] = useState('')
  const [started, setStarted] = useState(false)
  const [budget, setBudget] = useState(0)

  useEffect(() => {
    edgeWorkerApi.taskBudget().then(setBudget, () => undefined)
  }, [])

  if (picked.length === 0) return link.repos.length > 0 ? <p className="a-body-sm">{t('github_no_picked')}</p> : null

  const repo = picked.includes(chosen) ? chosen : picked.length === 1 ? picked[0] : ''
  const start = () =>
    run(async () => {
      if (!link.first_task_confirmed) await githubApi.firstTask()
      await createTaskClient(githubTask(repo, job))
      setJob('')
      setStarted(true)
    })

  return (
    <>
      <b className="a-body-b">{t('github_job_title')}</b>
      {picked.length > 1 && (
        <select aria-label={t('github_pick_repo')} className="a-input" value={repo} onChange={(e) => setChosen(e.target.value)}>
          <option value="">{t('github_pick_repo')}</option>
          {picked.map((name) => (
            <option key={name}>{name}</option>
          ))}
        </select>
      )}
      <textarea
        aria-label={t('github_job_hint')}
        className="a-input"
        rows={3}
        placeholder={t('github_job_example')}
        value={job}
        onChange={(e) => {
          setJob(e.target.value)
          setStarted(false)
        }}
      />
      <p className="a-body-sm">{t('github_budget_each', { budget: `$${budget.toFixed(BUDGET_DECIMALS)}` })}</p>
      <button type="button" className="a-btn secondary compact" disabled={!repo || !job.trim()} onClick={start}>
        {t('github_start')}
      </button>
      {started && <p className="a-body-sm">{t('github_started')}</p>}
    </>
  )
}

/** The way into HQ Coder on the web, in the same words and steps as the phone's sheet. */
export function GithubConnect() {
  const [link, setLink] = useState<GithubLink | null>(null)
  const [failed, setFailed] = useState(false)

  const run = useCallback<GithubRun>(async (work) => {
    try {
      await work?.()
      setLink(await githubApi.status())
      setFailed(false)
    } catch {
      setFailed(true)
    }
  }, [])

  useEffect(() => {
    void run()
  }, [run])

  const connect = () => run(async () => open((await githubApi.connect()).url))
  const disconnect = () => run(async () => open((await githubApi.disconnect()).settings_url))

  return (
    <section className="a-card mx-4 mb-3">
      <b className="a-body-b">{t('github_title')}</b>
      <p className="a-body-sm">{t('github_what_it_does')}</p>
      {failed && <ErrorText>{t('github_failed')}</ErrorText>}
      {!link && !failed && <p className="a-body-sm">{t('github_looking')}</p>}
      {link?.standing === 'not_connected' && (
        <>
          <p className="a-body-sm">{t('github_connect_body')}</p>
          <p className="a-body-sm a-warn-t">{t('github_choose_select')}</p>
          <p className="a-body-sm">{t('github_web_same_device')}</p>
          <button type="button" className="a-btn secondary compact" onClick={connect}>
            {t('github_connect')}
          </button>
        </>
      )}
      {link?.standing === 'needs_install' && (
        <>
          <p className="a-body-sm">{t('github_install_body')}</p>
          <p className="a-body-sm a-warn-t">{t('github_choose_select')}</p>
          <button type="button" className="a-btn secondary compact" onClick={() => open(link.install_url)}>
            {t('github_install')}
          </button>
        </>
      )}
      {link?.standing === 'reconnect' && (
        <>
          <p className="a-body-sm a-warn-t">{t('github_reconnect_body')}</p>
          <button type="button" className="a-btn secondary compact" onClick={connect}>
            {t('github_reconnect')}
          </button>
        </>
      )}
      {link?.standing === 'connected' && (
        <>
          <b className="a-body-b">{t('github_connected_as', { owner: link.owner ?? '' })}</b>
          {link.repository_selection === 'all' && <p className="a-body-sm a-warn-t">{t('github_all_repos')}</p>}
          {link.repos.length === 0 && <p className="a-body-sm">{t('github_no_repos')}</p>}
          <ul>
            {link.repos.map((repo) => (
              <Repo key={repo.report.repo} repo={repo} onAllow={() => run(async () => void (await githubApi.confirm(repo.report.repo)))} />
            ))}
          </ul>
          <JobForm link={link} run={run} />
        </>
      )}
      {link && link.standing !== 'not_connected' && (
        <button type="button" className="a-btn secondary compact" onClick={disconnect}>
          {t('github_disconnect')}
        </button>
      )}
    </section>
  )
}
