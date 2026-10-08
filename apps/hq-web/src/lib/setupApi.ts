import { hqJson } from './hqAuth'

/** The only provider the chat router reads a config key for on a fresh server. */
export const SETUP_PROVIDER = {
  label: 'OpenRouter (routes to any model)',
  keyUrl: 'https://openrouter.ai/keys',
}

export interface SetupStatus {
  needs_setup: boolean
  /** False when the server has no web token, so it refuses to take a key over HTTP. */
  available: boolean
}

export interface KeyTestResult {
  ok: boolean
  error?: string
}

export const fetchSetupStatus = () => hqJson<SetupStatus>('/api/setup/status')

export const testSetupKey = (apiKey: string) =>
  hqJson<KeyTestResult>('/api/setup/test', 'POST', { api_key: apiKey })

export const saveSetupKey = (apiKey: string) =>
  hqJson<{ ok: boolean }>('/api/setup/provider', 'POST', { api_key: apiKey })

/** Setup is offered only to a fresh install that can accept a key safely. */
export const shouldRedirectToSetup = (s: SetupStatus) => s.needs_setup && s.available
