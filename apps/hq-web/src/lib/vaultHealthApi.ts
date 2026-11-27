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
  /** Pairs of notes that say almost the same thing. Listed, not counted as damage. */
  similar: number
  /** How many notes were compared for that; 0 when the check did not run. */
  similar_checked: number
}
export interface SimilarPair {
  a: string
  b: string
  /** How much of their words they share, 0 to 100. */
  percent: number
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
  similar: SimilarPair[]
}
export interface HealthTrendPoint {
  ran_at: string
  problems: number
  links: number
  dead: number
}
/** How sure HQ is: `a` is certain, `b` is a good guess. */
export type FixTier = 'a' | 'b'
export type FixKind = 'stale_path' | 'renamed_note' | 'nested_frontmatter' | 'near_miss' | 'same_folder' | 'merge_duplicate'
/** One repair HQ could make. `before` and `after` are the link as written and as it would read. */
export interface HealthFix {
  id: string
  tier: FixTier
  kind: FixKind
  note: string
  line: number | null
  before: string
  after: string
}
export type SkipReason =
  | 'protected'
  | 'machine_written'
  | 'suite'
  | 'pinned'
  | 'agent_unreviewed'
  | 'recently_edited'
  | 'too_large'
  | 'cannot_relink'
  | 'changed'
  | 'conflict'
export interface SkippedNote {
  note: string
  why: SkipReason
}
/** One run of repairs. Undo works until `undone_at` is set. */
export interface HealthRepair {
  run_id: string
  ran_at: string
  source: 'owner' | 'auto'
  fixes: number
  rewritten: number
  trashed: number
  undone_at: string | null
  edited_since: number
}
export interface VaultHealth {
  checked: boolean
  checked_at: string | null
  report: HealthReport | null
  /** The first fixes the last check found (a screen's worth; `fix_counts` has the totals). */
  fixes: HealthFix[] | null
  fix_counts: { a: number; b: number } | null
  skipped: SkippedNote[] | null
  repairs: HealthRepair[]
  trend: HealthTrendPoint[]
}
/** What a repair run did. */
export interface AppliedRepair {
  run_id: string
  fixes: string[]
  rewritten: string[]
  trashed: string[]
  skipped: SkippedNote[]
}
export interface UndoneRepair {
  restored: string[]
  edited_since: string[]
  untrashed: string[]
  failed: string[]
}

/** What an owner can leave out of the check. The words match `hq_vault::health::kind`. */
export type IgnoreKind = 'dead_link' | 'ambiguous_link' | 'broken_anchor' | 'loose' | 'empty' | 'duplicate' | 'frontmatter' | 'similar'

export const getVaultHealth = () => hqJson<VaultHealth>('/api/vault/health')
export const scanVaultHealth = () => hqJson<VaultHealth>('/api/vault/health/scan', 'POST')
export const ignoreHealthItem = (kind: IgnoreKind, key: string) => hqJson<VaultHealth>('/api/vault/health/ignore', 'POST', { kind, key })
export const unignoreHealthItem = (kind: IgnoreKind, key: string) => hqJson<VaultHealth>('/api/vault/health/unignore', 'POST', { kind, key })

/** Do these fixes, or with no ids every certain one (`tier: 'a'`) or every one (`'all'`). */
export const applyHealthFixes = (body: { ids?: string[]; tier?: 'a' | 'all' }) =>
  hqJson<{ applied: AppliedRepair; health: VaultHealth }>('/api/vault/health/apply', 'POST', body)
/** Put back the notes of one run. Notes edited since are left as they are. */
export const undoHealthRepair = (runId: string) =>
  hqJson<{ undone: UndoneRepair; health: VaultHealth }>('/api/vault/health/undo', 'POST', { run_id: runId })

/** A repair made lately and not taken back: the card offers Undo for a day. */
export const UNDO_WINDOW_MS = 24 * 60 * 60 * 1000
export function undoableRepair(h: VaultHealth | undefined, nowMs: number = Date.now()): HealthRepair | null {
  const latest = h?.repairs?.[0]
  if (!latest || latest.undone_at || latest.fixes === 0) return null
  const at = Date.parse(latest.ran_at)
  return Number.isFinite(at) && nowMs - at < UNDO_WINDOW_MS ? latest : null
}

/** Things a person could fix. Notes nothing links to, and empty ones, are shown but are not damage. */
export function problemCount(c: HealthCounts): number {
  return c.links.dead + c.links.ambiguous + c.links.broken_anchors + c.duplicates + c.frontmatter
}
