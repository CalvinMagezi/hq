import { hqJson } from './hqAuth'

export type SetupProviderId = 'open_router' | 'anthropic' | 'google'

export interface SetupProviderInfo {
  id: SetupProviderId
  label: string
  keyUrl: string
}

/** Each key lands in the config field the chat router reads for that provider. */
export const SETUP_PROVIDERS: SetupProviderInfo[] = [
  { id: 'open_router', label: 'OpenRouter (routes to any model)', keyUrl: 'https://openrouter.ai/keys' },
  { id: 'anthropic', label: 'Anthropic (direct Claude access)', keyUrl: 'https://console.anthropic.com/settings/keys' },
  { id: 'google', label: 'Google AI (Gemini access)', keyUrl: 'https://aistudio.google.com/apikey' },
]

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

export const testSetupKey = (provider: SetupProviderId, apiKey: string) =>
  hqJson<KeyTestResult>('/api/setup/test', 'POST', { provider, api_key: apiKey })

export const saveSetupKey = (provider: SetupProviderId, apiKey: string) =>
  hqJson<{ ok: boolean }>('/api/setup/provider', 'POST', { provider, api_key: apiKey })

/** Setup is offered only to a fresh install that can accept a key safely. */
export const shouldRedirectToSetup = (s: SetupStatus) => s.needs_setup && s.available
