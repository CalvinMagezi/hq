import { hqJson } from './hqAuth'

export interface HelperRun {
  id: number
  step: 'install' | 'login' | 'check'
  state: 'running' | 'ok' | 'failed'
  tail: string
  urls: string[]
}

export interface SetupHelper {
  id: string
  title: string
  about: string
  free: string
  /** null until HQ has looked. */
  installed: boolean | null
  /** null when HQ cannot tell. */
  signed_in: boolean | null
  /** Unix seconds. */
  checked_at: number | null
  error: string | null
  run: HelperRun | null
}

export interface SetupHost {
  host: string
  helpers: SetupHelper[]
}

export const HELPER_SETUP_KEY = ['helper-setup']

export const fetchHelperSetup = (refresh = false) =>
  hqJson<{ hosts: SetupHost[] }>(`/api/helper-setup${refresh ? '?refresh=true' : ''}`).then((r) => r.hosts)

export type SetupAction = 'setup' | 'install' | 'login' | 'check'

export const runHelperSetup = (host: string, action: SetupAction, helpers: string[] = []) =>
  hqJson(`/api/helper-setup/${encodeURIComponent(host)}/${action}`, 'POST', { helpers })

export type HelperState = 'ready' | 'signed_out' | 'unknown_login' | 'missing' | 'unchecked'

export function helperState(h: SetupHelper): HelperState {
  if (h.installed === null) return 'unchecked'
  if (!h.installed) return 'missing'
  if (h.signed_in === true) return 'ready'
  if (h.signed_in === false) return 'signed_out'
  return 'unknown_login'
}

/** A step that is still running anywhere, so the panel keeps asking. */
export const anyRunning = (hosts: SetupHost[]) => hosts.some((h) => h.helpers.some((x) => x.run?.state === 'running'))
