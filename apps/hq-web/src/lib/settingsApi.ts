import { hqJson } from './hqAuth'

export interface SettingsBackend {
  name: string
  kind: string | null
  model: string | null
  effort: string | null
  endpoint_host: string | null
  primary: boolean
  fallback_position: number | null
}

export interface HqSettings {
  model: { active: string; default_model: string; relay_override: string | null; local_only: boolean }
  backends: { configured: boolean; primary: string; fallbacks: string[]; entries: SettingsBackend[] }
  provider_keys: { name: string; configured: boolean }[]
  limits: {
    chat_turn_timeout_secs: number
    turn_ack_timeout_secs: number
    background_turn_max_days: number
    background_progress_secs: number | null
  }
  safety: {
    bash_sandbox: string | null
    bash_network: boolean
    skills_write_approval: boolean
    web_allowed_origins: string[]
  }
  herdr: { default_host: string; hosts: string[]; drive_new_watches: boolean; driver_checkin_minutes: number; driver_nudge_budget: number; driver_no_progress_limit: number }
  integrations: {
    remote_mcp: { name: string; host: string; live_user_turn_only: boolean }[]
    searxng_host: string | null
    disk_watchdog_enabled: boolean
  }
}

export function fetchSettings(): Promise<HqSettings> {
  return hqJson('/api/settings')
}

const SECS_PER_MINUTE = 60
const SECS_PER_HOUR = 3600

/** Compact duration for a setting expressed in seconds. */
export function formatSeconds(secs: number): string {
  if (secs >= SECS_PER_HOUR && secs % SECS_PER_HOUR === 0) return `${secs / SECS_PER_HOUR} h`
  if (secs >= SECS_PER_MINUTE && secs % SECS_PER_MINUTE === 0) return `${secs / SECS_PER_MINUTE} min`
  return `${secs} s`
}
