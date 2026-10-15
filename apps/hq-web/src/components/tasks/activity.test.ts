import { expect, test } from 'bun:test'
import { ageLabel, authorLabel, freshnessLines, gapTitle, panelKind, parseStamp, reviewBadge, reviewSummary, unattributedCount } from './activity'
import type { ActivityNote } from '~/lib/taskActivityApi'

const NOW = Date.parse('2026-10-15T12:00:00Z')

const note = (over: Partial<ActivityNote> = {}): ActivityNote => ({
  path: 'Notebooks/a.md',
  attribution: 'recorded',
  link: null,
  type: null,
  actor: 'relay',
  source_kind: 'agent',
  review_state: 'unreviewed',
  ...over,
})

test('both timestamp shapes the API sends are read as UTC', () => {
  expect(parseStamp('2026-10-15 12:00:00').getTime()).toBe(NOW)
  expect(parseStamp('2026-10-15T12:00:00+00:00').getTime()).toBe(NOW)
  expect(parseStamp('2026-10-15T12:00:00.594019346+00:00').getTime()).toBe(NOW + 594)
  expect(parseStamp('2026-10-15T12:00:00Z').getTime()).toBe(NOW)
})

test('ages are coarse and a missing or unreadable time is unknown, never zero', () => {
  expect(ageLabel('2026-10-15 11:59:40', NOW)).toBe('just now')
  expect(ageLabel('2026-10-15 11:48:00', NOW)).toBe('12m ago')
  expect(ageLabel('2026-10-15 09:00:00', NOW)).toBe('3h ago')
  expect(ageLabel('2026-10-13 12:00:00', NOW)).toBe('2d ago')
  expect(ageLabel(null, NOW)).toBe('unknown')
  expect(ageLabel('garbage', NOW)).toBe('unknown')
})

test('accepted and unreviewed never look alike, and a version with no record is neither', () => {
  const accepted = reviewBadge({ review_state: 'accepted' })
  const unreviewed = reviewBadge({ review_state: 'unreviewed' })
  const none = reviewBadge({})
  expect(accepted.label).toBe('Accepted')
  expect(unreviewed.label).toBe('Unreviewed')
  expect(none.label).toBe('No record')
  expect(new Set([accepted.className, unreviewed.className, none.className]).size).toBe(3)
  expect(unreviewed.className).toContain('border-dashed')
  expect(accepted.className).not.toContain('border-dashed')
})

test('only a recorded author is named; every other case stays unknown', () => {
  expect(authorLabel(note())).toBe('relay (agent)')
  expect(authorLabel(note({ attribution: 'unattributed', actor: 'unknown' }))).toBe('unknown')
  expect(authorLabel(note({ attribution: 'no_record', actor: undefined }))).toBe('unknown')
  expect(authorLabel(note({ attribution: 'changed_outside_tools', actor: undefined }))).toContain('unknown')
  expect(authorLabel(note({ attribution: 'changed_outside_tools', actor: undefined }))).toContain('edited after')
  expect(authorLabel(note({ attribution: 'missing', actor: undefined }))).toBe('file is gone')
})

test('a note with an unknown source kind is shown by name only', () => {
  expect(authorLabel(note({ source_kind: 'unknown' }))).toBe('relay')
})

test('the panel is told apart for contributors, only-unattributed, and nothing', () => {
  const empty = { contributors: [], notes: [], holders: [], sessions: [] }
  expect(panelKind(empty)).toBe('none')
  expect(panelKind({ ...empty, notes: [note({ attribution: 'unattributed' })] })).toBe('unattributed_only')
  expect(
    panelKind({ ...empty, contributors: [{ actor: 'relay', source_kinds: ['agent'], notes: 1, leases: 0, last_active: null }] }),
  ).toBe('contributors')
})

test('unattributed notes are counted by every way an author can be missing', () => {
  const notes = [
    note(),
    note({ attribution: 'unattributed' }),
    note({ attribution: 'no_record' }),
    note({ attribution: 'changed_outside_tools' }),
    note({ attribution: 'missing' }),
  ]
  expect(unattributedCount(notes)).toBe(3)
})

test('review counts read in a fixed order and leave zeroes out', () => {
  expect(reviewSummary({ unreviewed: 2, accepted: 1, disputed: 0 })).toBe('1 accepted, 2 unreviewed')
  expect(reviewSummary({})).toBe('no reviews recorded')
})

test('a gap kind has a plain title and an unknown kind is shown as it came', () => {
  expect(gapTitle({ kind: 'note_unattributed' })).toBe('Author unknown')
  expect(gapTitle({ kind: 'some_new_gap' })).toBe('some new gap')
})

test('a source with nothing is named as having nothing, not given a time', () => {
  expect(freshnessLines({ task: '2026-10-15 11:00:00', notes: null }, NOW)).toEqual(['task: 1h ago', 'notes: nothing yet'])
})
