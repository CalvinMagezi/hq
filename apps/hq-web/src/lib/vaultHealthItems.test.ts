import { describe, expect, test } from 'bun:test'
import { fixFor, healthSections, skipText } from './vaultHealthItems'
import { problemCount, undoableRepair, type HealthFix, type HealthReport, type VaultHealth } from './vaultHealthApi'

const counts = (over: Partial<HealthReport['counts']['links']> = {}, rest: Partial<HealthReport['counts']> = {}) => ({
  notes: 20,
  unread: 0,
  links: { total: 60, ok: 50, dead: 0, ambiguous: 0, broken_anchors: 0, not_checked: 5, ...over },
  loose: 0,
  duplicates: 0,
  empty: 0,
  frontmatter: 0,
  similar: 0,
  similar_checked: 0,
  ...rest,
})

const empty: HealthReport = {
  counts: counts(),
  dead_links: [],
  ambiguous_links: [],
  broken_anchors: [],
  loose: [],
  duplicates: [],
  empty: [],
  frontmatter: [],
  unread: [],
  similar: [],
}

describe('healthSections', () => {
  test('a clean report has no sections and no problems', () => {
    expect(healthSections(empty)).toEqual([])
    expect(problemCount(empty.counts)).toBe(0)
  })

  test('what is broken comes first, and each item says where it is', () => {
    const report: HealthReport = {
      ...empty,
      counts: counts({ dead: 1, ambiguous: 1 }, { loose: 1 }),
      dead_links: [{ note: 'Notebooks/Work/A.md', line: 4, link: '[[Quarterly Reveiw]]', target: 'Quarterly Reveiw', did_you_mean: 'Quarterly Review' }],
      ambiguous_links: [{ note: 'Notebooks/B.md', line: 2, link: '[[Plan]]', candidates: ['Notebooks/X/Plan.md', 'Notebooks/Y/Plan.md'] }],
      loose: ['Notebooks/Lonely.md'],
    }
    const sections = healthSections(report)
    expect(sections.map((s) => s.id)).toEqual(['dead', 'ambiguous', 'loose'])
    expect(sections[0].items[0]).toMatchObject({
      note: 'Notebooks/Work/A.md',
      line: 4,
      text: '[[Quarterly Reveiw]]',
      detail: 'Did you mean Quarterly Review?',
      kind: 'dead_link',
    })
    expect(sections[1].items[0].detail).toBe('Could be Plan, Plan')
    expect(sections[2].items[0].line).toBeNull()
    expect(problemCount(report.counts)).toBe(2)
  })

  // The server names an item for ignoring by `key()` in crates/hq-vault/src/health.rs. If these
  // drift, ignoring would silently do nothing, so the formats are pinned here.
  test('ignore keys are the ones the server computes', () => {
    const report: HealthReport = {
      ...empty,
      counts: counts({ dead: 1, broken_anchors: 1 }, { duplicates: 1, frontmatter: 1, loose: 1 }),
      dead_links: [{ note: 'Notebooks/A.md', line: 1, link: '[[X]]', target: 'X', did_you_mean: null }],
      broken_anchors: [{ note: 'Notebooks/A.md', line: 2, link: '[[B#Gone]]', target_note: 'Notebooks/B.md', anchor: 'Gone' }],
      duplicates: [{ notes: ['Notebooks/One.md', 'Notebooks/Two.md'] }],
      frontmatter: [{ note: 'Notebooks/Bad.md', problem: 'invalid_yaml' }],
      loose: ['Notebooks/Lonely.md'],
    }
    const keys = Object.fromEntries(healthSections(report).map((s) => [s.items[0].kind, s.items[0].key]))
    expect(keys).toEqual({
      dead_link: 'Notebooks/A.md|[[X]]',
      broken_anchor: 'Notebooks/A.md|[[B#Gone]]',
      duplicate: 'Notebooks/One.md|Notebooks/Two.md',
      frontmatter: 'Notebooks/Bad.md',
      loose: 'Notebooks/Lonely.md',
    })
  })

  test('the total is the exact count even when only some examples are listed', () => {
    const report: HealthReport = {
      ...empty,
      counts: counts({ dead: 120 }),
      dead_links: Array.from({ length: 50 }, (_, i) => ({ note: 'Notebooks/A.md', line: i + 1, link: `[[Gone ${i}]]`, target: `Gone ${i}`, did_you_mean: null })),
    }
    const [dead] = healthSections(report)
    expect(dead.total).toBe(120)
    expect(dead.items).toHaveLength(50)
    expect(new Set(dead.items.map((i) => i.id)).size).toBe(50)
  })

  test('each kind of property problem has its own sentence', () => {
    const report: HealthReport = {
      ...empty,
      counts: counts({}, { frontmatter: 3 }),
      frontmatter: [
        { note: 'Notebooks/A.md', problem: 'unclosed' },
        { note: 'Notebooks/B.md', problem: 'invalid_yaml' },
        { note: 'Notebooks/C.md', problem: 'nested' },
      ],
    }
    const details = healthSections(report)[0].items.map((i) => i.detail)
    expect(new Set(details).size).toBe(3)
  })
})

const fix = (over: Partial<HealthFix> = {}): HealthFix => ({
  id: 'f1',
  tier: 'a',
  kind: 'stale_path',
  note: 'Notebooks/A.md',
  line: 1,
  before: '[[Old/Plan]]',
  after: '[[New/Plan]]',
  ...over,
})

describe('fixFor', () => {
  const base = { id: 'x', note: 'Notebooks/A.md', line: 1, text: '[[Old/Plan]]', detail: null, key: 'k' } as const

  test('a link finds its repair by the note it is in and how it is written', () => {
    const fixes = [fix({ id: 'other', before: '[[Else]]' }), fix(), fix({ id: 'elsewhere', note: 'Notebooks/B.md' })]
    expect(fixFor({ ...base, kind: 'dead_link' }, fixes)?.id).toBe('f1')
    expect(fixFor({ ...base, kind: 'ambiguous_link' }, [fix({ kind: 'same_folder' })])?.id).toBe('f1')
    expect(fixFor({ ...base, kind: 'dead_link', text: '[[Nothing Like It]]' }, fixes)).toBeNull()
  })

  test('a repeated note finds its repair by any note in the set, and properties by their note', () => {
    const merge = fix({ id: 'm', kind: 'merge_duplicate', note: 'Notebooks/Dup.md', before: 'Notebooks/Dup.md', after: 'Notebooks/Keep.md' })
    expect(fixFor({ ...base, kind: 'duplicate', key: 'Notebooks/Keep.md|Notebooks/Dup.md' }, [merge])?.id).toBe('m')
    expect(fixFor({ ...base, kind: 'duplicate', key: 'Notebooks/X.md|Notebooks/Y.md' }, [merge])).toBeNull()
    const props = fix({ id: 'p', kind: 'nested_frontmatter', before: '', after: '' })
    expect(fixFor({ ...base, kind: 'frontmatter' }, [props])?.id).toBe('p')
  })

  test('notes nothing links to and near repeats are never repaired', () => {
    expect(fixFor({ ...base, kind: 'loose' }, [fix()])).toBeNull()
    expect(fixFor({ ...base, kind: 'similar' }, [fix()])).toBeNull()
  })

  test('sections carry the repair beside the item it fixes', () => {
    const report: HealthReport = {
      ...empty,
      counts: counts({ dead: 2 }),
      dead_links: [
        { note: 'Notebooks/A.md', line: 1, link: '[[Old/Plan]]', target: 'Old/Plan', did_you_mean: null },
        { note: 'Notebooks/A.md', line: 2, link: '[[Nowhere]]', target: 'Nowhere', did_you_mean: null },
      ],
    }
    const [dead] = healthSections(report, [fix()])
    expect(dead.items.map((i) => i.fix?.id ?? null)).toEqual(['f1', null])
  })
})

describe('notes that say almost the same thing', () => {
  test('are listed with how much they share, and are not damage', () => {
    const report: HealthReport = {
      ...empty,
      counts: counts({}, { similar: 1, similar_checked: 40 }),
      similar: [{ a: 'Notebooks/A.md', b: 'Notebooks/B.md', percent: 91 }],
    }
    expect(problemCount(report.counts)).toBe(0)
    const [sim] = healthSections(report)
    expect(sim.id).toBe('similar')
    expect(sim.items[0]).toMatchObject({ text: 'A, B', detail: '91 percent of the words are the same', key: 'Notebooks/A.md|Notebooks/B.md', kind: 'similar' })
  })
})

describe('skipText', () => {
  test('every reason has its own words', () => {
    const reasons = ['protected', 'machine_written', 'suite', 'pinned', 'agent_unreviewed', 'recently_edited', 'too_large', 'cannot_relink', 'changed', 'conflict'] as const
    const words = reasons.map(skipText)
    expect(new Set(words).size).toBe(reasons.length)
    expect(words.every((w) => w.length > 0 && !w.includes('notes_health'))).toBe(true)
  })
})

describe('undoableRepair', () => {
  const run = (over: Partial<VaultHealth['repairs'][number]> = {}) => ({
    run_id: 'r1',
    ran_at: new Date(Date.now() - 60 * 60_000).toISOString(),
    source: 'owner' as const,
    fixes: 3,
    rewritten: 2,
    trashed: 0,
    undone_at: null,
    edited_since: 0,
    ...over,
  })
  const health = (repairs: VaultHealth['repairs']): VaultHealth => ({ checked: true, checked_at: null, report: empty, fixes: [], fix_counts: null, skipped: [], repairs, trend: [] })

  test('is the latest run while it is recent and not taken back', () => {
    expect(undoableRepair(health([run()]))?.run_id).toBe('r1')
    expect(undoableRepair(health([run({ undone_at: 'x' })]))).toBeNull()
    expect(undoableRepair(health([run({ ran_at: new Date(Date.now() - 3 * 86_400_000).toISOString() })]))).toBeNull()
    expect(undoableRepair(health([run({ fixes: 0 })]))).toBeNull()
    expect(undoableRepair(health([]))).toBeNull()
    expect(undoableRepair(undefined)).toBeNull()
  })
})

