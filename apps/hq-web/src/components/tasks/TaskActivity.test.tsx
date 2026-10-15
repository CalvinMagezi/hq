import { expect, test } from 'bun:test'
import { renderToString } from 'react-dom/server'
import type { InitiativeActivity, TaskActivity } from '~/lib/taskActivityApi'
import { InitiativeBriefingView } from './InitiativeBriefing'
import { TaskActivityView } from './TaskActivity'

const NOW = Date.parse('2026-10-15T12:00:00Z')

const base = (): TaskActivity => ({
  scope: 'task',
  generated_at: '2026-10-15T12:00:00+00:00',
  basis: 'Generated now',
  task: {
    id: 'tk-1',
    display_id: 'WRK-001',
    title: 'A task',
    status: 'in_progress',
    priority: null,
    assignees: [],
    blocked_by: [],
    updated_at: '2026-10-15 11:00:00',
  },
  awaiting_review: false,
  blocker: null,
  checkpoint: null,
  holders: [],
  recent_leases: [],
  sessions: [],
  notes: [],
  notes_total: 0,
  review_counts: {},
  decisions_and_findings: {},
  open_questions: [],
  contributors: [],
  gaps: [],
  gaps_total: 0,
  freshness: { task: '2026-10-15 11:00:00', notes: null },
  sources_with_nothing: ['notes'],
})

const withContributors = (): TaskActivity => ({
  ...base(),
  holders: [
    {
      lease_id: 'ws-1',
      actor: 'relay',
      harness: 'native',
      branch: '',
      harness_session: null,
      started_at: '2026-10-15 11:00:00',
      last_heartbeat_at: '2026-10-15 11:55:00',
      ended_at: null,
      end_reason: null,
      live: true,
    },
  ],
  contributors: [{ actor: 'relay', source_kinds: ['agent'], notes: 2, leases: 1, last_active: '2026-10-15T11:50:00+00:00' }],
  notes: [
    {
      path: 'Notebooks/plan.md',
      attribution: 'recorded',
      link: 'produced',
      type: 'decision',
      actor: 'relay',
      source_kind: 'agent',
      session: 's-1',
      written_at: '2026-10-15T11:50:00+00:00',
      review_state: 'accepted',
      reviewed_by: 'owner',
      review_reason: 'checked against the source',
    },
    {
      path: 'Notebooks/draft.md',
      attribution: 'recorded',
      link: null,
      type: null,
      actor: 'relay',
      source_kind: 'agent',
      session: 's-1',
      written_at: '2026-10-15T11:40:00+00:00',
      review_state: 'unreviewed',
    },
  ],
  notes_total: 2,
  review_counts: { accepted: 1, unreviewed: 1 },
  decisions_and_findings: {
    accepted: [
      { path: 'Notebooks/plan.md', attribution: 'recorded', link: 'produced', type: 'decision', actor: 'relay', review_state: 'accepted' },
    ],
  },
  gaps: [{ kind: 'note_without_session', detail: 'written by an agent but no session was recorded', subject: 'Notebooks/x.md' }],
  gaps_total: 1,
  freshness: { task: '2026-10-15 11:00:00', notes: '2026-10-15T11:50:00+00:00' },
  sources_with_nothing: [],
})

const unattributedOnly = (): TaskActivity => ({
  ...base(),
  notes: [
    { path: 'Notebooks/found.md', attribution: 'unattributed', link: 'related', type: null },
    { path: 'Notebooks/edited.md', attribution: 'changed_outside_tools', link: null, type: null },
  ],
  notes_total: 2,
  gaps: [
    { kind: 'note_unattributed', detail: 'the version on disk has no known author', subject: 'Notebooks/found.md' },
    { kind: 'note_changed_outside_tools', detail: 'the file was changed after its last recorded write', subject: 'Notebooks/edited.md' },
    { kind: 'in_progress_without_a_holder', detail: 'the status is in_progress but no live lease or running session is attached', subject: 'WRK-001' },
  ],
  gaps_total: 3,
})

/** React puts `<!-- -->` between adjacent text nodes; the words are what matter here. */
const plain = (markup: string) => markup.replaceAll('<!-- -->', '')

const html = (view: TaskActivity) => plain(renderToString(<TaskActivityView view={view} nowMs={NOW} />))

test('a task with contributors shows who, their notes, review state and the holder', () => {
  const out = html(withContributors())
  expect(out).toContain('data-kind="contributors"')
  expect(out).toContain('Held by')
  expect(out).toContain('relay on native, last heartbeat 5m ago')
  expect(out).toContain('2 notes, 1 lease')
  expect(out).toContain('Accepted')
  expect(out).toContain('Unreviewed')
  expect(out).toContain('Decision by owner: checked against the source')
  expect(out).toContain('Decisions and findings')
  expect(out).toContain('No session recorded')
})

test('a task with nothing recorded says so and invents nothing', () => {
  const out = html(base())
  expect(out).toContain('data-kind="none"')
  expect(out).toContain('Nothing has been recorded on this task')
  expect(out).not.toContain('Contributors')
  expect(out).not.toContain('Held by')
  expect(out).toContain('notes: nothing yet')
})

test('a task with only unattributed changes keeps every author unknown and lists the gaps', () => {
  const out = html(unattributedOnly())
  expect(out).toContain('data-kind="unattributed_only"')
  expect(out).toContain('none has a known author')
  expect(out).toContain('2 with no known author')
  expect(out).toContain('By unknown')
  expect(out).toContain('edited after its last recorded write')
  expect(out).toContain('Nobody holds it')
  expect(out).toContain('No record')
  expect(out).not.toContain('Contributors')
})

test('an edit time is labelled as when a version was recorded, never as progress', () => {
  const out = html(withContributors())
  expect(out).toContain('version recorded')
  expect(out).toContain('A note being edited is not progress')
  expect(out.toLowerCase()).not.toContain('last worked')
})

test('gaps beyond the first few fold away but are still reachable', () => {
  const view = base()
  view.gaps = Array.from({ length: 6 }, (_, i) => ({ kind: 'note_unattributed', detail: 'no author', subject: `Notebooks/n${i}.md` }))
  view.gaps_total = 6
  const out = html(view)
  expect(out).toContain('Gaps (6)')
  expect(out).toContain('2 more')
  expect(out).toContain('Notebooks/n5.md')
})

const initiative = (over: Partial<InitiativeActivity> = {}): InitiativeActivity => ({
  scope: 'initiative',
  generated_at: '2026-10-15T12:00:00+00:00',
  basis: 'Generated now',
  initiative: { id: 'in-1', name: 'Work', prefix: 'WRK', space: null },
  tasks_shown: 2,
  status_counts: { in_progress: 1, ready_for_review: 1 },
  tasks: [],
  awaiting_review: [{ id: 'tk-2', display_id: 'WRK-002', title: 'Check it' }],
  blocked: [{ id: 'tk-3', display_id: 'WRK-003', title: 'Stuck', reason: 'waiting on the bank', waiting_on: null }],
  review_counts: { accepted: 1, unreviewed: 2 },
  recent_notes: [
    {
      path: 'Notebooks/a.md',
      attribution: 'recorded',
      link: null,
      type: null,
      actor: 'relay',
      source_kind: 'agent',
      written_at: '2026-10-15T11:50:00+00:00',
      review_state: 'unreviewed',
      task_display_id: 'WRK-001',
    },
  ],
  contributors: [{ actor: 'relay', source_kinds: ['agent'], notes: 1, leases: 1, last_active: '2026-10-15T11:50:00+00:00' }],
  gaps_total: 2,
  gap_kinds: { note_unattributed: 2 },
  freshness: { tasks: '2026-10-15 11:00:00' },
  ...over,
})

test('the briefing shows counts, review waits, recorded blockers, contributors and gaps', () => {
  const out = plain(renderToString(<InitiativeBriefingView view={initiative()} nowMs={NOW} />))
  expect(out).toContain('1 in progress, 1 ready for review')
  expect(out).toContain('Waiting for review')
  expect(out).toContain('WRK-002')
  expect(out).toContain('waiting on the bank')
  expect(out).toContain('1 accepted, 2 unreviewed')
  expect(out).toContain('2 gaps')
  expect(out).toContain('WRK-001, by relay (agent), version recorded 10m ago')
})

test('a briefing with no contributors says no one is on record, and an empty initiative says there is nothing to brief', () => {
  const quiet = plain(renderToString(<InitiativeBriefingView view={initiative({ contributors: [], recent_notes: [] })} nowMs={NOW} />))
  expect(quiet).toContain('No one is on record')
  const none = plain(renderToString(<InitiativeBriefingView view={initiative({ tasks_shown: 0 })} nowMs={NOW} />))
  expect(none).toContain('nothing to brief')
})
