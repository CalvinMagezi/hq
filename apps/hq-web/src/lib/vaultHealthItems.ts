import { noteName } from '~/components/vault/noteRows'
import { t, type PluralKey, type StringKey } from '~/strings'
import type { HealthFix, HealthReport, IgnoreKind, SkipReason } from './vaultHealthApi'

/** One thing to look at: where it is, what it says, and the name the server knows it by. */
export interface HealthItem {
  /** Unique within its section; also the React key. */
  id: string
  /** The note to open for it. */
  note: string
  /** The line in that note, when it has one. */
  line: number | null
  /** The link or the names involved. */
  text: string
  /** A sentence of help under it, when there is one. */
  detail: string | null
  kind: IgnoreKind
  /** Names the item for ignoring. Must match `key()` in crates/hq-vault/src/health.rs. */
  key: string
  /** The repair HQ could make for it, when there is one. */
  fix: HealthFix | null
}

export interface HealthSection {
  id: 'dead' | 'ambiguous' | 'anchors' | 'duplicates' | 'frontmatter' | 'similar' | 'loose'
  /** Plural string whose `{n}` is the exact count, shown as the section's heading. */
  titleKey: PluralKey
  total: number
  items: HealthItem[]
}

const FRONTMATTER_TEXT: Record<string, StringKey> = {
  unclosed: 'notes_health_fm_unclosed',
  invalid_yaml: 'notes_health_fm_invalid',
  nested: 'notes_health_fm_nested',
}

const pairKey = (note: string, link: string) => `${note}|${link}`

const LINK_FIXES: readonly string[] = ['stale_path', 'renamed_note', 'near_miss', 'same_folder']

/** The repair for an item, if there is one: a link by the note it is in and how it is written, a repeat by the notes in its set. */
export function fixFor(item: Omit<HealthItem, 'fix'>, fixes: readonly HealthFix[]): HealthFix | null {
  switch (item.kind) {
    case 'dead_link':
    case 'ambiguous_link':
      return fixes.find((f) => LINK_FIXES.includes(f.kind) && f.note === item.note && f.before === item.text) ?? null
    case 'duplicate':
      return fixes.find((f) => f.kind === 'merge_duplicate' && item.key.split('|').includes(f.note)) ?? null
    case 'frontmatter':
      return fixes.find((f) => f.kind === 'nested_frontmatter' && f.note === item.note) ?? null
    default:
      return null
  }
}

const SKIP_TEXT: Record<SkipReason, StringKey> = {
  protected: 'notes_health_skip_protected',
  machine_written: 'notes_health_skip_machine',
  suite: 'notes_health_skip_suite',
  pinned: 'notes_health_skip_pinned',
  agent_unreviewed: 'notes_health_skip_agent',
  recently_edited: 'notes_health_skip_recent',
  too_large: 'notes_health_skip_large',
  cannot_relink: 'notes_health_skip_relink',
  changed: 'notes_health_skip_changed',
  conflict: 'notes_health_skip_conflict',
}

/** Why a note was left alone, in words. */
export const skipText = (why: SkipReason): string => t(SKIP_TEXT[why] ?? 'notes_health_skip_changed')

/** The report as sections, in the order that matters: what is broken first. Empty sections are left out. */
export function healthSections(r: HealthReport, fixes: readonly HealthFix[] = []): HealthSection[] {
  const c = r.counts
  const withFix = (items: Omit<HealthItem, 'fix'>[]): HealthItem[] => items.map((i) => ({ ...i, fix: fixFor(i, fixes) }))
  const sections: HealthSection[] = [
    {
      id: 'dead',
      titleKey: 'notes_health_dead',
      total: c.links.dead,
      items: withFix(r.dead_links.map((d) => ({
        id: pairKey(d.note, d.link) + d.line,
        note: d.note,
        line: d.line,
        text: d.link,
        detail: d.did_you_mean ? t('notes_health_did_you_mean', { name: d.did_you_mean }) : null,
        kind: 'dead_link',
        key: pairKey(d.note, d.link),
      }))),
    },
    {
      id: 'ambiguous',
      titleKey: 'notes_health_ambiguous',
      total: c.links.ambiguous,
      items: withFix(r.ambiguous_links.map((a) => ({
        id: pairKey(a.note, a.link) + a.line,
        note: a.note,
        line: a.line,
        text: a.link,
        detail: t('notes_health_could_be', { names: a.candidates.map(noteName).join(', ') }),
        kind: 'ambiguous_link',
        key: pairKey(a.note, a.link),
      }))),
    },
    {
      id: 'anchors',
      titleKey: 'notes_health_anchors',
      total: c.links.broken_anchors,
      items: withFix(r.broken_anchors.map((b) => ({
        id: pairKey(b.note, b.link) + b.line,
        note: b.note,
        line: b.line,
        text: b.link,
        detail: t('notes_health_missing_in', { name: noteName(b.target_note) }),
        kind: 'broken_anchor',
        key: pairKey(b.note, b.link),
      }))),
    },
    {
      id: 'duplicates',
      titleKey: 'notes_health_duplicates',
      total: c.duplicates,
      items: withFix(r.duplicates.map((g) => ({
        id: g.notes.join('|'),
        note: g.notes[0],
        line: null,
        text: g.notes.map(noteName).join(', '),
        detail: null,
        kind: 'duplicate',
        key: g.notes.join('|'),
      }))),
    },
    {
      id: 'frontmatter',
      titleKey: 'notes_health_frontmatter',
      total: c.frontmatter,
      items: withFix(r.frontmatter.map((f) => ({
        id: f.note,
        note: f.note,
        line: null,
        text: noteName(f.note),
        detail: t(FRONTMATTER_TEXT[f.problem] ?? 'notes_health_fm_invalid'),
        kind: 'frontmatter',
        key: f.note,
      }))),
    },
    {
      id: 'similar',
      titleKey: 'notes_health_similar',
      total: c.similar,
      items: withFix(
        r.similar.map((p) => ({
          id: `${p.a}|${p.b}`,
          note: p.a,
          line: null,
          text: `${noteName(p.a)}, ${noteName(p.b)}`,
          detail: t('notes_health_same_words', { n: p.percent }),
          kind: 'similar' as const,
          key: `${p.a}|${p.b}`,
        })),
      ),
    },
    {
      id: 'loose',
      titleKey: 'notes_health_loose',
      total: c.loose,
      items: withFix(r.loose.map((p) => ({ id: p, note: p, line: null, text: noteName(p), detail: null, kind: 'loose', key: p }))),
    },
  ]
  return sections.filter((s) => s.total > 0)
}
