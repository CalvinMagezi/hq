import { hqJson } from './hqAuth'

export type SetupProviderId = 'open_router' | 'anthropic' | 'google'

export interface SetupProviderInfo {
  id: SetupProviderId
  name: 'setup_openrouter_name' | 'setup_anthropic_name' | 'setup_google_name'
  line: 'setup_openrouter_line' | 'setup_anthropic_line' | 'setup_google_line'
  keyUrl: string
  help: 'setup_key_help_openrouter' | 'setup_key_help_anthropic' | 'setup_key_help_google'
}

/** Each key lands in the config field the chat router reads for that provider. */
export const SETUP_PROVIDERS: SetupProviderInfo[] = [
  { id: 'open_router', keyUrl: 'https://openrouter.ai/keys', name: 'setup_openrouter_name', line: 'setup_openrouter_line', help: 'setup_key_help_openrouter' },
  { id: 'anthropic', keyUrl: 'https://console.anthropic.com/settings/keys', name: 'setup_anthropic_name', line: 'setup_anthropic_line', help: 'setup_key_help_anthropic' },
  { id: 'google', keyUrl: 'https://aistudio.google.com/apikey', name: 'setup_google_name', line: 'setup_google_line', help: 'setup_key_help_google' },
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

export type KeyAnswer = 'refused' | 'no_credit' | 'too_busy' | 'no_answer'

/** The service's raw error as one plain answer: never a status code on screen. */
export function keyAnswer(raw: string | undefined): KeyAnswer {
  const text = (raw ?? '').toLowerCase()
  if (/\b(401|403)\b|unauthori[sz]ed|invalid|incorrect|rejected|forbidden|refused/.test(text)) return 'refused'
  if (/\b402\b|credit|quota|billing|insufficient|balance/.test(text)) return 'no_credit'
  if (/\b(429|503|529)\b|rate.?limit|too many|overload|busy|capacity/.test(text)) return 'too_busy'
  return 'no_answer'
}

export const KEY_ANSWER_STRING = {
  refused: 'setup_refused',
  no_credit: 'setup_no_credit',
  too_busy: 'setup_too_busy',
  no_answer: 'setup_no_answer',
} as const satisfies Record<KeyAnswer, string>

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

/** A server with no web sign-in refuses a key from a browser: the page says what to run instead. */
export const showsNoSignIn = (s: SetupStatus) => s.needs_setup && !s.available

export type KeyCheck = { accepted: true } | { accepted: false; answer: KeyAnswer }

/** Asks the service about the key and saves nothing. */
export async function checkSetupKey(provider: SetupProviderId, apiKey: string): Promise<KeyCheck> {
  try {
    const result = await testSetupKey(provider, apiKey)
    return result.ok ? { accepted: true } : { accepted: false, answer: keyAnswer(result.error) }
  } catch (err) {
    return { accepted: false, answer: keyAnswer(err instanceof Error ? err.message : undefined) }
  }
}
