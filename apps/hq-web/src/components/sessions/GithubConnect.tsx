import { useCallback, useEffect, useState } from 'react'
import { ErrorText } from '~/components/sessions/ErrorText'
import { githubApi, type GithubLink, type GithubRepo } from '~/lib/github'
import { t } from '~/strings'

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

/** The way into HQ Coder on the web, in the same words and steps as the phone's sheet. */
export function GithubConnect() {
  const [link, setLink] = useState<GithubLink | null>(null)
  const [failed, setFailed] = useState(false)

  const run = useCallback(async (work?: () => Promise<void>) => {
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
          {!link.first_task_confirmed && (
            <button type="button" className="a-btn secondary compact" onClick={() => run(async () => void (await githubApi.firstTask()))}>
              {t('github_first_task')}
            </button>
          )}
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
