// The notes check, the owner's side: HQ reads every note for links that lead nowhere, notes that
// repeat each other and properties it cannot read. The server is `/api/vault/health`
// (crates/hq-web/src/vault_health_api.rs); the report is `hq_vault::health::Report`.
import { hqJson } from './hqAuth'

export interface DeadLink {
  note: string
  line: number
  link: string
  target: string
  did_you_mean: string | null
}
export interface AmbiguousLink {
  note: string
  line: number
  link: string
  candidates: string[]
}
export interface BrokenAnchor {
  note: string
  line: number
  link: string
  target_note: string
  anchor: string
}
export interface DuplicateGroup {
  notes: string[]
}
export type FrontmatterProblem = 'unclosed' | 'invalid_yaml' | 'nested'
export interface FrontmatterIssue {
  note: string
  problem: FrontmatterProblem
}
export interface LinkCounts {
  total: number
  ok: number
  dead: number
  ambiguous: number
  broken_anchors: number
  not_checked: number
}
export interface HealthCounts {
  notes: number
  unread: number
  links: LinkCounts
  loose: number
  duplicates: number
  empty: number
  frontmatter: number
}
/** Each list holds the first 50 examples; the counts are exact. */
export interface HealthReport {
  counts: HealthCounts
  dead_links: DeadLink[]
  ambiguous_links: AmbiguousLink[]
  broken_anchors: BrokenAnchor[]
  loose: string[]
  duplicates: DuplicateGroup[]
  empty: string[]
  frontmatter: FrontmatterIssue[]
  unread: string[]
}
export interface HealthTrendPoint {
  ran_at: string
  problems: number
  links: number
  dead: number
}
export interface VaultHealth {
  checked: boolean
  checked_at: string | null
  report: HealthReport | null
  trend: HealthTrendPoint[]
}

/** What an owner can leave out of the check. The words match `hq_vault::health::kind`. */
export type IgnoreKind = 'dead_link' | 'ambiguous_link' | 'broken_anchor' | 'loose' | 'empty' | 'duplicate' | 'frontmatter'

export const getVaultHealth = () => hqJson<VaultHealth>('/api/vault/health')
export const scanVaultHealth = () => hqJson<VaultHealth>('/api/vault/health/scan', 'POST')
export const ignoreHealthItem = (kind: IgnoreKind, key: string) => hqJson<VaultHealth>('/api/vault/health/ignore', 'POST', { kind, key })
export const unignoreHealthItem = (kind: IgnoreKind, key: string) => hqJson<VaultHealth>('/api/vault/health/unignore', 'POST', { kind, key })

/** Things a person could fix. Notes nothing links to, and empty ones, are shown but are not damage. */
export function problemCount(c: HealthCounts): number {
  return c.links.dead + c.links.ambiguous + c.links.broken_anchors + c.duplicates + c.frontmatter
}
