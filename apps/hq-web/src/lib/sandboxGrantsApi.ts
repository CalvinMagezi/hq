import { hqJson } from './hqAuth'
import type { NotificationItem } from './notificationsApi'
import { t, type StringKey } from '~/strings'

export type SandboxCapability = 'fs_write' | 'fs_read' | 'loopback' | 'egress' | 'profile' | string
export type SandboxGrantState = 'pending' | 'active' | 'declined' | 'refused' | 'revoked' | 'expired' | string
export type SandboxDecidedBy = 'policy' | 'owner' | 'system' | string
export type SandboxGrantScope = 'session' | 'task' | 'repo' | string
export type SandboxStarterName = 'mobile-dev' | 'node-build' | 'rust-build' | string

export interface SandboxGrant {
  id: string
  session_id: string
  task_id: string | null
  host: string
  repo: string
  capability: SandboxCapability
  resource: string
  state: SandboxGrantState
  decided_by: SandboxDecidedBy
  helper_reason: string
  decision_note: string
  requested_at: number
  decided_at: number | null
  applied_at: number | null
  expires_at: number | null
  revoked_at: number | null
  scope?: SandboxGrantScope
  suggested_starter?: SandboxStarterName | null
}

export interface SandboxGrantsResponse {
  grants: SandboxGrant[]
}

export interface SandboxRulesResponse {
  rules: SandboxGrant[]
}

export interface SandboxGrantProfile {
  name: string
  writable: string[]
  readable: string[]
  loopback_ports: number[]
  egress: string[]
}

export interface SandboxProfilesResponse {
  profiles: SandboxGrantProfile[]
}

export interface SandboxStartersResponse {
  starters: SandboxGrantProfile[]
}

export interface SandboxGrantsSummary {
  approved_by_policy: number
  approved_by_owner: number
  asked_owner: number
  declined: number
  refused: number
  standing_rules: number
}

export interface SandboxSummaryResponse {
  days: number
  summary: SandboxGrantsSummary
}

export interface BindSandboxProfileInput {
  scope: 'session' | 'task'
  id: string
  name: string
}

export const SANDBOX_GRANTS_SOURCE = 'sandbox-grants'
export const SANDBOX_GRANTS_QUERY_KEY = ['sandbox-grants'] as const
export const SANDBOX_PROFILES_QUERY_KEY = ['sandbox-grants-profiles'] as const
export const SANDBOX_RULES_QUERY_KEY = ['sandbox-grants-rules'] as const
export const SANDBOX_STARTERS_QUERY_KEY = ['sandbox-grants-starters'] as const
export const SANDBOX_SUMMARY_QUERY_KEY = ['sandbox-grants-summary'] as const
export const NO_PROFILE = 'none'

const TASK_SCOPE_PREFIX = 'task:'
const REPO_SCOPE_PREFIX = 'repo:'

export const listSandboxGrants = (session?: string) =>
  hqJson<SandboxGrantsResponse>(
    session ? `/api/sandbox-grants?session=${encodeURIComponent(session)}` : '/api/sandbox-grants',
  )

export const listSandboxRules = () =>
  hqJson<SandboxRulesResponse>('/api/sandbox-grants/rules')

export const listSandboxStarters = () =>
  hqJson<SandboxStartersResponse>('/api/sandbox-grants/starters')

export const getSandboxSummary = (days = 7) =>
  hqJson<SandboxSummaryResponse>(`/api/sandbox-grants/summary?days=${encodeURIComponent(String(days))}`)

export const revokeSandboxGrant = (id: string) =>
  hqJson<{ revoked: string }>(`/api/sandbox-grants/${encodeURIComponent(id)}/revoke`, 'POST')

export const listSandboxProfiles = () =>
  hqJson<SandboxProfilesResponse>('/api/sandbox-grants/profiles')

export const bindSandboxProfile = (input: BindSandboxProfileInput) =>
  hqJson<{ bound: string }>('/api/sandbox-grants/profile', 'POST', input)

export function isSandboxGrantNotification(item: Pick<NotificationItem, 'metadata'>): boolean {
  return item.metadata.extra?.source_task === SANDBOX_GRANTS_SOURCE
}

export function isResourceGrant(grant: Pick<SandboxGrant, 'capability'>): boolean {
  return grant.capability !== 'profile'
}

export function isStandingRule(grant: Pick<SandboxGrant, 'scope' | 'session_id'>): boolean {
  return (
    grant.scope === 'task' ||
    grant.scope === 'repo' ||
    grant.session_id.startsWith(TASK_SCOPE_PREFIX) ||
    grant.session_id.startsWith(REPO_SCOPE_PREFIX)
  )
}

export function activeResourceGrants(grants: SandboxGrant[]): SandboxGrant[] {
  return grants.filter((g) => isResourceGrant(g) && !isStandingRule(g) && g.state === 'active')
}

export function pendingResourceGrants(grants: SandboxGrant[]): SandboxGrant[] {
  return grants.filter((g) => isResourceGrant(g) && !isStandingRule(g) && g.state === 'pending')
}

export function historyResourceGrants(grants: SandboxGrant[]): SandboxGrant[] {
  return grants.filter((g) => isResourceGrant(g) && g.state !== 'active' && g.state !== 'pending')
}

/** The profile currently bound to a task, or 'none' when none is active. */
export function taskProfileFromGrants(grants: SandboxGrant[], taskId: string): string {
  const scope = `${TASK_SCOPE_PREFIX}${taskId}`
  const match = grants.find((g) => g.capability === 'profile' && g.state === 'active' && g.session_id === scope)
  return match?.resource || NO_PROFILE
}

export function capabilityModeKey(capability: string): StringKey {
  switch (capability) {
    case 'fs_write':
      return 'grants_mode_fs_write'
    case 'fs_read':
      return 'grants_mode_fs_read'
    case 'loopback':
      return 'grants_mode_loopback'
    case 'egress':
      return 'grants_mode_egress'
    default:
      return 'grants_mode_other'
  }
}

export function grantStateKey(state: string): StringKey {
  switch (state) {
    case 'active':
      return 'grants_state_active'
    case 'pending':
      return 'grants_state_pending'
    case 'revoked':
      return 'grants_state_revoked'
    case 'expired':
      return 'grants_state_expired'
    case 'refused':
      return 'grants_state_refused'
    case 'declined':
      return 'grants_state_declined'
    default:
      return 'grants_state_revoked'
  }
}

export function starterButtonLabel(name: SandboxStarterName): string {
  switch (name) {
    case 'mobile-dev':
      return t('grants_allow_starter_mobile_dev')
    case 'node-build':
      return t('grants_allow_starter_node_build')
    case 'rust-build':
      return t('grants_allow_starter_rust_build')
    default:
      return t('grants_allow_starter_generic', { starter: name })
  }
}

export function starterDescription(name: SandboxStarterName): string {
  switch (name) {
    case 'mobile-dev':
      return t('grants_starter_mobile_dev')
    case 'node-build':
      return t('grants_starter_node_build')
    case 'rust-build':
      return t('grants_starter_rust_build')
    default:
      return name
  }
}

/** Plain-English summary of a standing rule, keeping the session computer's `~` as written. */
export function formatStandingRule(rule: Pick<SandboxGrant, 'scope' | 'session_id' | 'task_id' | 'repo' | 'capability' | 'resource'>): string {
  const isRepo = rule.scope === 'repo' || rule.session_id.startsWith(REPO_SCOPE_PREFIX)
  if (isRepo) {
    const repo = rule.repo || rule.session_id.replace(/^repo:/, '')
    switch (rule.capability) {
      case 'fs_write':
        return t('grants_rule_repo_fs_write', { repo, resource: rule.resource })
      case 'fs_read':
        return t('grants_rule_repo_fs_read', { repo, resource: rule.resource })
      case 'loopback':
        return t('grants_rule_repo_loopback', { repo, resource: rule.resource })
      case 'egress':
        return t('grants_rule_repo_egress', { repo, resource: rule.resource })
      default:
        return t('grants_rule_repo_other', { repo, resource: rule.resource })
    }
  }
  const task = rule.task_id || rule.session_id.replace(/^task:/, '')
  switch (rule.capability) {
    case 'fs_write':
      return t('grants_rule_task_fs_write', { task, resource: rule.resource })
    case 'fs_read':
      return t('grants_rule_task_fs_read', { task, resource: rule.resource })
    case 'loopback':
      return t('grants_rule_task_loopback', { task, resource: rule.resource })
    case 'egress':
      return t('grants_rule_task_egress', { task, resource: rule.resource })
    default:
      return t('grants_rule_task_other', { task, resource: rule.resource })
  }
}

export function formatSandboxSummary(days: number, summary: SandboxGrantsSummary): string {
  if (summary.approved_by_policy === 1) {
    return t('grants_settings_summary_one', {
      days,
      asked: summary.asked_owner,
      refused: summary.refused,
    })
  }
  return t('grants_settings_summary', {
    days,
    allowed: summary.approved_by_policy,
    asked: summary.asked_owner,
    refused: summary.refused,
  })
}

export interface ParsedSandboxGrant {
  title: string
  sessionId: string | null
  taskId: string | null
  repo: string
  capability: SandboxCapability
  resource: string
  helperReason: string
  suggestedStarter: SandboxStarterName | null
}

const DESCRIBE_PATTERNS: [RegExp, SandboxCapability][] = [
  [/^write to (.+)$/, 'fs_write'],
  [/^read (.+)$/, 'fs_read'],
  [/^connect to local port (.+)$/, 'loopback'],
  [/^reach (.+)$/, 'egress'],
]

function parseDescribePhrase(phrase: string): { capability: SandboxCapability; resource: string } {
  const clean = phrase.trim().replace(/\.$/, '')
  for (const [re, cap] of DESCRIBE_PATTERNS) {
    const m = clean.match(re)
    if (m) return { capability: cap, resource: m[1].trim() }
  }
  return { capability: 'other', resource: clean }
}

/** Extracts the resource, mode, session, repo, suggested starter and helper's own words from a sandbox-grants notification. */
export function parseSandboxGrantNotification(
  item: Pick<NotificationItem, 'title' | 'description' | 'metadata'>,
  grants?: SandboxGrant[],
): ParsedSandboxGrant {
  const desc = item.description ?? ''
  const sessionMatch = desc.match(/Helper session (\S+) asked to (.+?)\.\s+HQ could not/)
  const sessionId = sessionMatch?.[1] ?? item.metadata.extra?.session_id ?? null

  const phraseFromDesc = sessionMatch?.[2]
  const phraseFromTitle = item.title.replace(/^A helper wants to\s+/i, '')
  const parsed = parseDescribePhrase(phraseFromDesc ?? phraseFromTitle)

  const wordsMatch = desc.match(/The helper's own words, not HQ's:\s*"([\s\S]*?)"\.\s*Dismiss to refuse\.?/)
  const parsedReason = wordsMatch?.[1] ?? ''

  const matchedGrant = grants?.find(
    (g) =>
      isResourceGrant(g) &&
      !isStandingRule(g) &&
      g.state === 'pending' &&
      (!sessionId || g.session_id === sessionId) &&
      (!parsed.resource || g.resource === parsed.resource),
  )

  return {
    title: item.title,
    sessionId: matchedGrant?.session_id ?? sessionId,
    taskId: matchedGrant?.task_id ?? item.metadata.extra?.task_id ?? null,
    repo: matchedGrant?.repo ?? '',
    capability: matchedGrant?.capability ?? parsed.capability,
    resource: matchedGrant?.resource ?? parsed.resource,
    helperReason: matchedGrant?.helper_reason ?? parsedReason,
    suggestedStarter: matchedGrant?.suggested_starter ?? null,
  }
}
