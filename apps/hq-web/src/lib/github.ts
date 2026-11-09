// Connecting GitHub: the owner routes behind the web token (crates/hq-web/src/github_connect_api.rs).

import { hqJson } from '~/lib/hqAuth'

export interface GithubReport {
  repo: string
  secrets: string[]
  previews: string[]
  has_ci: boolean
  protected: boolean
}

export interface GithubRepo {
  flagged: boolean
  picked: boolean
  report: GithubReport
}

export type GithubStanding = 'not_connected' | 'needs_install' | 'connected' | 'reconnect'

export interface GithubLink {
  standing: GithubStanding
  owner: string | null
  repository_selection: string | null
  install_url: string | null
  settings_url: string | null
  first_task_confirmed: boolean
  repos: GithubRepo[]
}

export const githubApi = {
  status: () => hqJson<GithubLink>('/api/github'),
  connect: () => hqJson<{ url: string }>('/api/github/connect', 'POST'),
  disconnect: () => hqJson<{ settings_url: string | null }>('/api/github/disconnect', 'POST'),
  confirm: (repo: string) => hqJson<{ confirmed: boolean }>('/api/github/repos/confirm', 'POST', { repo }),
  firstTask: () => hqJson<{ confirmed: boolean }>('/api/github/first-task', 'POST'),
}
