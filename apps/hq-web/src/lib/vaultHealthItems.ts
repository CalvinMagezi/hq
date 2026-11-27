import { noteName } from '~/components/vault/noteRows'
import { t, type PluralKey, type StringKey } from '~/strings'
import type { HealthReport, IgnoreKind } from './vaultHealthApi'

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
}

export interface HealthSection {
  id: 'dead' | 'ambiguous' | 'anchors' | 'duplicates' | 'frontmatter' | 'loose'
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

/** The report as sections, in the order that matters: what is broken first. Empty sections are left out. */
export function healthSections(r: HealthReport): HealthSection[] {
  const c = r.counts
  const sections: HealthSection[] = [
    {
      id: 'dead',
      titleKey: 'notes_health_dead',
      total: c.links.dead,
      items: r.dead_links.map((d) => ({
        id: pairKey(d.note, d.link) + d.line,
        note: d.note,
        line: d.line,
        text: d.link,
        detail: d.did_you_mean ? t('notes_health_did_you_mean', { name: d.did_you_mean }) : null,
        kind: 'dead_link',
        key: pairKey(d.note, d.link),
      })),
    },
    {
      id: 'ambiguous',
      titleKey: 'notes_health_ambiguous',
      total: c.links.ambiguous,
      items: r.ambiguous_links.map((a) => ({
        id: pairKey(a.note, a.link) + a.line,
        note: a.note,
        line: a.line,
        text: a.link,
        detail: t('notes_health_could_be', { names: a.candidates.map(noteName).join(', ') }),
        kind: 'ambiguous_link',
        key: pairKey(a.note, a.link),
      })),
    },
    {
      id: 'anchors',
      titleKey: 'notes_health_anchors',
      total: c.links.broken_anchors,
      items: r.broken_anchors.map((b) => ({
        id: pairKey(b.note, b.link) + b.line,
        note: b.note,
        line: b.line,
        text: b.link,
        detail: t('notes_health_missing_in', { name: noteName(b.target_note) }),
        kind: 'broken_anchor',
        key: pairKey(b.note, b.link),
      })),
    },
    {
      id: 'duplicates',
      titleKey: 'notes_health_duplicates',
      total: c.duplicates,
      items: r.duplicates.map((g) => ({
        id: g.notes.join('|'),
        note: g.notes[0],
        line: null,
        text: g.notes.map(noteName).join(', '),
        detail: null,
        kind: 'duplicate',
        key: g.notes.join('|'),
      })),
    },
    {
      id: 'frontmatter',
      titleKey: 'notes_health_frontmatter',
      total: c.frontmatter,
      items: r.frontmatter.map((f) => ({
        id: f.note,
        note: f.note,
        line: null,
        text: noteName(f.note),
        detail: t(FRONTMATTER_TEXT[f.problem] ?? 'notes_health_fm_invalid'),
        kind: 'frontmatter',
        key: f.note,
      })),
    },
    {
      id: 'loose',
      titleKey: 'notes_health_loose',
      total: c.loose,
      items: r.loose.map((p) => ({ id: p, note: p, line: null, text: noteName(p), detail: null, kind: 'loose', key: p })),
    },
  ]
  return sections.filter((s) => s.total > 0)
}
