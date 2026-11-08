import { expect, test } from 'bun:test'
import { ageLabel, authorLabel, gapLine, lastActivityLine, panelKind, parseStamp, reviewBadge, reviewSummary, unattributedCount } from './activity'
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
  expect(ageLabel('2026-10-15 11:59:40', NOW)).toBe('now')
  expect(ageLabel('2026-10-15 11:48:00', NOW)).toBe('12 min ago')
  expect(ageLabel('2026-10-15 09:00:00', NOW)).toBe('3 h ago')
  expect(ageLabel('2026-10-13 12:00:00', NOW)).toBe('2 days ago')
  expect(ageLabel(null, NOW)).toBe('Not known')
  expect(ageLabel('garbage', NOW)).toBe('Not known')
})

test('accepted and unreviewed never look alike, and a version with no record is neither', () => {
  const accepted = reviewBadge({ review_state: 'accepted' })
  const unreviewed = reviewBadge({ review_state: 'unreviewed' })
  const none = reviewBadge({})
  expect(accepted.label).toBe('Accepted')
  expect(unreviewed.label).toBe('Unreviewed')
  expect(none.label).toBe('No record')
  expect(new Set([accepted.className, unreviewed.className, none.className]).size).toBe(3)
  expect(unreviewed.className).toContain('l-dashed')
  expect(accepted.className).not.toContain('l-dashed')
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

test('a gap reads as one plain sentence, and anything unknown keeps the server words', () => {
  expect(gapLine({ kind: 'note_unattributed', subject: 'Notebooks/a.md', detail: 'raw' })).toBe('Nobody is on record as writing Notebooks/a.md.')
  expect(gapLine({ kind: 'in_progress_without_a_holder', subject: 'WRK-1', detail: 'raw' })).toBe('It says In progress, but no helper is working on it.')
  // A note gap with no note named, or a kind this app does not know, keeps the server's own words.
  expect(gapLine({ kind: 'note_missing', detail: 'the file is gone' })).toBe('the file is gone')
  expect(gapLine({ kind: 'some_new_gap', detail: 'something new' })).toBe('something new')
})

test('the closing line names the newest activity and never lists internal sources', () => {
  expect(lastActivityLine({ task: '2026-10-15 11:00:00', notes: '2026-10-15T11:30:00+00:00', leases: null }, NOW)).toBe(
    'Last activity 30 min ago. Editing a note is not progress: the status comes from the task.',
  )
  expect(lastActivityLine({ task: null, notes: null }, NOW)).toBe('Editing a note is not progress: the status comes from the task.')
})
