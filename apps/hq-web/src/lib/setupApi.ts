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

/** Checks the key with the provider and saves it only if it works, replacing any earlier key. */
export const replaceProviderKey = (provider: SetupProviderId, apiKey: string) =>
  hqJson<{ ok: boolean; provider: SetupProviderId }>('/api/settings/provider-key', 'POST', { provider, api_key: apiKey })

export type KeyState = 'env' | 'configured' | 'not_set'

/** Where a provider's key stands. A key from the server's environment wins over the config file. */
export function keyState(row: { configured: boolean; source?: 'config' | 'env' | null }): KeyState {
  if (row.source === 'env') return 'env'
  return row.configured ? 'configured' : 'not_set'
}

/** Setup is offered only to a fresh install that can accept a key safely. */
export const shouldRedirectToSetup = (s: SetupStatus) => s.needs_setup && s.available
