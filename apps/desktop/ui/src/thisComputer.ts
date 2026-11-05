// What the This computer tab says for each report from the host. Pure: no page, no Tauri, so
// `bun test` runs it. Every sentence comes from the shared catalogue.
import { t, tn } from '../../../hq-web/src/strings'

export type Lockdown = { reason: string; source: string; at: number }
export interface Report {
  state: 'stopped' | 'starting' | 'running' | 'external' | 'exited'
  code?: number | null
  start: { step: number; steps: number; elapsed_secs: number; stalled: boolean; restarting: boolean } | null
  exited_at: number | null
  you_stopped_at: number | null
  last_run: { started_at: number; ended_at: number } | null
  usual_secs: number | null
  program: { path: string; version: string | null } | null
  serves_hub: boolean
  hub_web: boolean
  free_bytes: number | null
  almost_full: boolean
  lockdown: Lockdown | null
  helpers: { reachable: boolean; working: string[]; needs_you: string[] }
}

export type Kind =
  | 'not_found'
  | 'locked'
  | 'port_held'
  | 'restarting'
  | 'starting'
  | 'slow'
  | 'on'
  | 'external'
  | 'stopped'
  | 'you_stopped'
  | 'off'
  | 'phone_off'
  | 'phone_on'

export type Tone = 'offline' | 'working' | 'ready' | 'needs'

export interface View {
  kind: Kind
  title: string
  state: string
  tone: Tone
  lines: string[]
  /** Open HQ stays off, and says why. */
  openWhy: string | null
}

const STEP_KEYS = ['hub_step_1', 'hub_step_2', 'hub_step_3', 'hub_step_4'] as const

export function stepName(step: number): string {
  return t(STEP_KEYS[Math.min(Math.max(step, 1), 4) - 1])
}

/** How long ago, in the catalogue's relative-time words. */
export function agoText(secs: number): string {
  const mins = Math.floor(secs / 60)
  if (mins < 1) return t('time_just_now')
  if (mins < 60) return t('time_min_ago', { n: mins })
  const hours = Math.floor(mins / 60)
  if (hours < 24) return t('time_hour_ago', { n: hours })
  const days = Math.floor(hours / 24)
  return days === 1 ? t('time_yesterday') : tn('time_days_ago', days)
}

export function clock(seconds: number): string {
  return new Date(seconds * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

export function kindOf(r: Report, portHeld: boolean): Kind {
  if (!r.program) return 'not_found'
  if (r.lockdown) return 'locked'
  if (portHeld) return 'port_held'
  if (r.state === 'starting' || r.state === 'running') {
    if (r.start?.restarting && r.state === 'starting') return 'restarting'
    if (r.state === 'starting') return r.start?.stalled ? 'slow' : 'starting'
    return r.serves_hub ? 'phone_on' : 'on'
  }
  if (r.state === 'external') return 'external'
  if (r.state === 'exited') return 'stopped'
  if (r.you_stopped_at !== null) return 'you_stopped'
  return r.serves_hub ? 'phone_off' : 'off'
}

/** Who used Stop every helper, in the person's words. `hub` is the paired hub's name, if one is known. */
export function lockdownWho(source: string, hub: string | null): string {
  if (source === 'cli') return t('hub_lockdown_who_this')
  if (source === 'breaker') return t('hub_lockdown_who_safety')
  return hub ?? t('hub_lockdown_who_phone')
}

export function describe(r: Report, portHeld: boolean, hub: string | null): View {
  const kind = kindOf(r, portHeld)
  const base = { kind, lines: [] as string[], openWhy: t('hub_open_why_off') }
  const starting = r.start
  switch (kind) {
    case 'not_found':
      return { ...base, title: t('hub_not_found_title'), state: t('hub_not_found_state'), tone: 'needs', lines: [t('hub_not_found_note')] }
    case 'locked': {
      const l = r.lockdown as Lockdown
      return {
        ...base,
        title: t('hub_lockdown_title'),
        state: t('hub_lockdown_state'),
        tone: 'needs',
        lines: [
          t('hub_lockdown_used', { time: clock(l.at), who: lockdownWho(l.source, hub) }),
          t('hub_lockdown_reason', { text: l.reason }),
          t('hub_lockdown_note'),
        ],
      }
    }
    case 'port_held':
      return {
        ...base,
        title: t('hub_port_held_title'),
        state: t('hub_port_held_state'),
        tone: 'needs',
        lines: [t('reason_port_held'), t('hub_port_held_hint')],
        openWhy: t('hub_port_held_wait'),
      }
    case 'restarting':
      return { ...base, title: t('state_hub_restarting'), state: t('hub_restarting_state'), tone: 'working', openWhy: t('hub_open_why_starting') }
    case 'starting':
    case 'slow': {
      const lines = [
        t('hub_starting_step', {
          step: starting?.step ?? 1,
          steps: starting?.steps ?? 4,
          secs: starting?.elapsed_secs ?? 0,
          step_name: stepName(starting?.step ?? 1),
        }),
      ]
      if (r.usual_secs) lines.push(t('hub_starting_usual', { secs: r.usual_secs }))
      if (kind === 'slow') lines.push(t('hub_slow_note'), t('hub_stopped_at', { step: stepName(starting?.step ?? 1) }))
      return {
        ...base,
        title: t('state_starting', { seconds: starting?.elapsed_secs ?? 0 }),
        state: t('hub_starting_state'),
        tone: kind === 'slow' ? 'needs' : 'working',
        lines,
        openWhy: t('hub_open_why_starting'),
      }
    }
    case 'on':
      return { ...base, title: t('hub_title'), state: t('hub_on_state'), tone: 'ready', lines: [t('hub_on_note')], openWhy: null }
    case 'phone_on':
      return {
        ...base,
        title: t('hub_phone_off_title'),
        state: t('hub_on_state'),
        tone: 'ready',
        lines: [tn('hub_phone_on_note', r.helpers.working.length)],
        openWhy: null,
      }
    case 'external':
      return { ...base, title: t('hub_title'), state: t('hub_on_state'), tone: 'ready', lines: [t('hub_external_note'), t('hub_external_how')], openWhy: null }
    case 'stopped': {
      const at = r.exited_at ? clock(r.exited_at) : ''
      const ran = r.last_run ? Math.round((r.last_run.ended_at - r.last_run.started_at) / 60) : 0
      return {
        ...base,
        title: t('state_hub_stopped'),
        state: t('hub_stopped_state'),
        tone: 'needs',
        lines: [
          ran >= 1 ? t('hub_stopped_note', { time: at, mins: ran }) : t('hub_stopped_note_soon', { time: at }),
          r.code == null ? t('hub_exit_no_code', { time: at }) : t('hub_exit_code', { code: r.code, time: at }),
        ],
      }
    }
    case 'you_stopped':
      return {
        ...base,
        title: t('state_hub_you_stopped'),
        state: t('hub_off_state'),
        tone: 'offline',
        lines: [t('hub_you_stopped_note', { time: clock(r.you_stopped_at ?? 0) })],
      }
    case 'phone_off':
      return {
        ...base,
        title: t('hub_phone_off_title'),
        state: t('hub_off_state'),
        tone: 'offline',
        lines: [t('hub_phone_off_note', { device: hub ?? t('hub_lockdown_who_phone') })],
        openWhy: t('hub_open_why_phone'),
      }
    default:
      return { ...base, title: t('hub_off_title'), state: t('hub_off_state'), tone: 'offline', lines: [t('hub_off_note'), t('hub_off_sub')] }
  }
}

/** Whether Stop needs the red dialog: helpers are working here and stop with it. */
export function stopIsRisky(r: Report): boolean {
  return r.helpers.working.length > 0
}

export function sizeText(bytes: number): string {
  const mb = Math.max(1, Math.round(bytes / (1024 * 1024)))
  return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`
}
