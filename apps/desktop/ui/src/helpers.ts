// What the Helpers tab and a helper's page say. Pure: no page, no Tauri, so `bun test` runs it.
import { t, type StringKey } from '../../../hq-web/src/strings'

export type Group = 'coding' | 'general' | 'app'
export type State =
  | 'unchecked'
  | 'checking'
  | 'not_installed'
  | 'program_not_found'
  | 'check_failed'
  | 'needs_signin'
  | 'needs_you'
  | 'not_connected'
  | 'update_available'
  | 'ready'
  | 'trying'
  | 'waiting'

export interface HelperView {
  id: string
  name: string
  group: Group
  path: string | null
  state: State
  version: string | null
  version_from_package: boolean
  latest: string | null
  new_here: boolean
  can_connect: boolean
  connected: boolean
  can_sign_in: boolean
  install: string | null
}

export type Action =
  | 'connect'
  | 'signin'
  | 'see_asked'
  | 'choose'
  | 'check_again'
  | 'update'
  | 'try'
  | 'install'

const LABEL: Record<State, StringKey> = {
  unchecked: 'hh_state_unchecked',
  checking: 'hh_state_checking',
  not_installed: 'hh_state_not_installed',
  program_not_found: 'hh_state_program_not_found',
  check_failed: 'hh_state_check_failed',
  needs_signin: 'hh_state_needs_signin',
  needs_you: 'hh_state_needs_you',
  not_connected: 'hh_state_not_connected',
  update_available: 'hh_state_update',
  ready: 'hh_state_ready',
  trying: 'hh_state_trying',
  waiting: 'hh_state_waiting',
}

export const stateLabel = (s: State): string => t(LABEL[s])

// Amber says the person has something to do; blue says it is working; grey is fine.
export function tone(s: State): 'amber' | 'blue' | 'grey' {
  if (['needs_signin', 'needs_you', 'program_not_found', 'check_failed', 'not_connected'].includes(s)) return 'amber'
  return s === 'checking' || s === 'trying' ? 'blue' : 'grey'
}

const RANK: Record<State, number> = {
  needs_you: 0,
  needs_signin: 0,
  program_not_found: 0,
  check_failed: 0,
  not_connected: 0,
  checking: 1,
  waiting: 1,
  trying: 1,
  unchecked: 2,
  update_available: 2,
  ready: 2,
  not_installed: 3,
}

// Rows that need you first, then the rest, then what is not installed. Names keep their order within a rank.
export function orderRows(rows: HelperView[]): HelperView[] {
  return rows
    .map((r, i) => ({ r, i }))
    .sort((a, b) => RANK[a.r.state] - RANK[b.r.state] || a.i - b.i)
    .map((x) => x.r)
}

export function rowLine(v: HelperView, newHere = false): string {
  const version = v.version ?? ''
  const base = (() => {
    switch (v.state) {
      case 'not_installed':
        return t('hh_line_not_here')
      case 'program_not_found':
        return t('hh_line_not_found')
      case 'check_failed':
        return t('hh_line_failed')
      case 'needs_you':
        return t('hh_line_needs_you')
      case 'needs_signin':
        return version ? t('hh_line_signin', { version }) : t('hh_state_needs_signin')
      case 'not_connected':
        return version ? t('hh_line_not_connected', { version }) : t('hh_line_not_connected_nov')
      case 'update_available':
        return t('hh_line_update', { version, latest: v.latest ?? '' })
      case 'checking':
        return t('hh_line_reading')
      case 'unchecked':
        return t('hh_line_unchecked')
      case 'ready':
      case 'trying':
      case 'waiting':
        return !version
          ? t('hh_line_no_version')
          : v.connected && v.can_connect
            ? t('hh_line_connected', { version })
            : t('hh_line_version', { version })
    }
  })()
  return newHere && v.path ? `${base} · ${t('hh_line_new_here', { path: v.path })}` : base
}

// The one button a row offers, or none.
export function rowAction(v: HelperView): Action | null {
  switch (v.state) {
    case 'not_installed':
      return 'install'
    case 'program_not_found':
      return 'choose'
    case 'check_failed':
      return 'check_again'
    case 'needs_signin':
      return 'signin'
    case 'needs_you':
      return 'see_asked'
    case 'not_connected':
      return v.can_connect ? 'connect' : null
    case 'update_available':
      return 'update'
    case 'ready':
      return v.group === 'app' ? null : 'try'
    default:
      return null
  }
}

export const ACTION_KEY: Record<Action, StringKey> = {
  connect: 'hh_act_connect',
  signin: 'hh_act_signin',
  see_asked: 'hh_act_see_asked',
  choose: 'hh_act_choose',
  check_again: 'hh_act_check_again',
  update: 'hh_act_update',
  try: 'hh_act_try',
  install: 'hh_act_install',
}

export function grouped(rows: HelperView[]): Record<Group, HelperView[]> {
  const out: Record<Group, HelperView[]> = { coding: [], general: [], app: [] }
  for (const r of orderRows(rows)) out[r.group].push(r)
  return out
}

export const nothingFound = (rows: HelperView[]): boolean =>
  rows.filter((r) => r.group !== 'app').every((r) => r.state === 'not_installed')

// What a try's outcome means for the row: a sign-in link shown means it wants a sign-in.
export function afterTry(outcome: { state: State; link_ready: boolean }): State {
  if (outcome.link_ready) return 'needs_signin'
  return outcome.state === 'trying' ? 'ready' : outcome.state
}

export function minutesLeft(startedMs: number, nowMs: number, limitSecs = 600): string {
  const left = Math.max(0, limitSecs - Math.floor((nowMs - startedMs) / 1000))
  return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`
}

// A line the person can copy to answer a question in the Try it folder.
export const askedCommand = (dir: string, program: string): string => t('hd_try_asked_cmd', { dir, program })

export const shortHome = (path: string, home: string): string =>
  home && path.startsWith(`${home}/`) ? `~${path.slice(home.length)}` : path
