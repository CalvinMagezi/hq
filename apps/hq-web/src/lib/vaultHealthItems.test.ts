import { describe, expect, test } from 'bun:test'
import { healthSections } from './vaultHealthItems'
import { problemCount, type HealthReport } from './vaultHealthApi'

const counts = (over: Partial<HealthReport['counts']['links']> = {}, rest: Partial<HealthReport['counts']> = {}) => ({
  notes: 20,
  unread: 0,
  links: { total: 60, ok: 50, dead: 0, ambiguous: 0, broken_anchors: 0, not_checked: 5, ...over },
  loose: 0,
  duplicates: 0,
  empty: 0,
  frontmatter: 0,
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
