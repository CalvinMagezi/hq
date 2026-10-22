import { expect, test } from 'bun:test'
import type { ReactElement } from 'react'
import { renderToString as ssr } from 'react-dom/server'
import type {
  AuditCard,
  AuditStack,
  Briefing,
  Calibration,
  QueueHealth,
  Rollout,
  SweepDetail,
  TaskVerification,
  VerificationStatus,
} from '~/lib/verificationApi'
import { AuditCardView, AuditStackView } from './AuditSheet'
import { BriefingCardView } from './BriefingCard'
import { QueueHealthPanel } from './QueueHealthPanel'
import { RolloutPanel } from './RolloutPanel'
import { ScopePickerView, SweepBodyView } from './SweepSheet'
import { TaskVerificationView } from './TaskVerification'
import { VerificationBarView } from './VerificationBar'

/** The HTML React renders, without the comment markers it puts between adjacent text nodes. */
const renderToString = (node: ReactElement) => ssr(node).replaceAll('<!-- -->', '')

const NOW = 1_800_000_000_000

const health = (over: Partial<QueueHealth> = {}): QueueHealth => ({
  mode: 'enforce',
  backlog: {},
  owner_by_kind: {},
  oldest_owner_hours: null,
  verdicts_24h: 0,
  completed_24h: 0,
  est_clear_hours: null,
  deferrals: [],
  spent_this_month_usd: 0,
  monthly_ceiling_usd: null,
  alerts: [],
  ...over,
})

const calibration = (over: Partial<Calibration> = {}): Calibration => ({
  verifier_config: 'premium',
  canaries: { runs: 0, defects: 0, caught: 0, controls: 0, passed: 0, catch_rate: null, false_reject_rate: null },
  tiers: [],
  open_audits: 0,
  ...over,
})

const rollout = (over: Partial<Rollout> = {}): Rollout => ({
  stage: 'shadow',
  stage_label: 'Shadow: verdicts are recorded and nothing moves',
  next: { label: 'Enforce tier 0 (notes, research, planning)', change: 'tasks.verification.mode: enforce, with enforce_tiers: [0]' },
  gates: [
    { label: 'Enough tier 0 verdicts compared with what happened next', met: false, detail: '4 of 30 compared: 4 agreed' },
    { label: 'The backlog has been read by a sweep', met: true, detail: 'nothing is waiting for review' },
  ],
  ready: false,
  warnings: [],
  shadow: [],
  ...over,
})

const status = (over: Partial<VerificationStatus> = {}): VerificationStatus => ({
  mode: 'enforce',
  autonomy_gate: 'demote',
  confirm_over: 25,
  calibration: calibration(),
  queue: health({ backlog: { owner: 3, verifier: 12, ci: 2 } }),
  rollout: rollout(),
  holds: 0,
  active_sweeps: 0,
  ...over,
})

const noop = () => {}

test('the bar counts who tasks wait on with the owner first, and offers the three things the owner does', () => {
  const html = renderToString(
    <VerificationBarView status={status()} healthOpen={false} onToggleHealth={noop} onVerifyNow={noop} onAudit={noop} />
  )
  expect(html.indexOf('3 needs you')).toBeGreaterThan(-1)
  expect(html.indexOf('3 needs you')).toBeLessThan(html.indexOf('12 queued'))
  expect(html).toContain('2 checks running')
  expect(html).toContain('Verify now')
  expect(html).toContain('Audit')
  expect(html).not.toContain('watching only')
  expect(html).not.toContain('Waiting for budget')
})

test('in shadow mode the bar says it only watches, and an empty queue says so', () => {
  const html = renderToString(
    <VerificationBarView
      status={status({ mode: 'shadow', queue: health({ backlog: {} }), calibration: calibration({ open_audits: 4 }) })}
      healthOpen={false}
      onToggleHealth={noop}
      onVerifyNow={noop}
      onAudit={noop}
    />
  )
  expect(html).toContain('watching only')
  expect(html).toContain('Nothing waiting')
  expect(html).toContain('Audit (4)')
})

test('the queue panel puts alerts first and says why anything is deferred', () => {
  const html = renderToString(
    <QueueHealthPanel
      health={health({
        backlog: { owner: 8 },
        alerts: ['8 items wait on you, over 5'],
        deferrals: [{ waiting_for: 'budget', reason: 'the monthly reviewer ceiling is used up', tasks: 3 }],
      })}
    />
  )
  expect(html.indexOf('8 items wait on you, over 5')).toBeLessThan(html.indexOf('completed by HQ'))
  expect(html).toContain('the monthly reviewer ceiling is used up')
  expect(html).toContain('3</span> tasks (waiting for budget)')
  expect(renderToString(<QueueHealthPanel health={health()} />)).toContain('Nothing is waiting on anyone.')
})

test('the rollout section says where verification is, puts the unmet conditions first and names the one setting', () => {
  const html = renderToString(<QueueHealthPanel health={health()} rollout={rollout()} />)
  expect(html).toContain('Turning it on')
  expect(html).toContain('Shadow: verdicts are recorded and nothing moves')
  expect(html).toContain('One condition is not met yet.')
  expect(html.indexOf('Enough tier 0 verdicts')).toBeLessThan(html.indexOf('The backlog has been read by a sweep'))
  expect(html).toContain('tasks.verification.mode: enforce, with enforce_tiers: [0]')
})

test('a rollout with every condition met says so, and one with nothing next says nothing about a next step', () => {
  const ready = renderToString(<RolloutPanel rollout={rollout({ ready: true, gates: [] })} />)
  expect(ready).toContain('Every condition is met. This is the one setting to change.')
  const done = renderToString(
    <RolloutPanel rollout={rollout({ stage: 'all_tiers', stage_label: 'Enforcing every tier', next: null, gates: [], ready: true })} />
  )
  expect(done).toContain('Enforcing every tier')
  expect(done).not.toContain('Next:')
  expect(done).not.toContain('Setting:')
})

test('a withdrawn calibration shows as a warning above the next step', () => {
  const html = renderToString(
    <RolloutPanel rollout={rollout({ warnings: ['tier 0 is enforced but its calibration is withdrawn (3 of 20 audits disagreed)'] })} />
  )
  expect(html.indexOf('calibration is withdrawn')).toBeLessThan(html.indexOf('Next:'))
})

test('the bar opens onto the rollout when the queue panel is open', () => {
  const html = renderToString(
    <VerificationBarView status={status()} healthOpen onToggleHealth={noop} onVerifyNow={noop} onAudit={noop} />
  )
  expect(html).toContain('Turning it on')
})

const briefing = (over: Partial<Briefing> = {}): Briefing => ({
  since: '2026-10-21 00:00:00',
  mode: 'enforce',
  needs_you: [],
  needs_you_total: 0,
  bulk_offer: null,
  completed: [],
  completed_total: 0,
  reworked: [],
  reworked_total: 0,
  veto_windows: [],
  audit_pool: 0,
  open_audits: 0,
  health: health(),
  ...over,
})

const waitingItem = (n: number) => ({
  task_id: `tk-${n}`,
  display_id: `WRK-00${n}`,
  title: `Task ${n}`,
  initiative_id: 'in-1',
  tier: 1,
  who: 'owner' as const,
  kind: 'held',
  reason: 'held for you',
  since: '2026-10-20 10:00:00',
})

test('a weekend of completions is a line per initiative and never a pass by pass list', () => {
  const html = renderToString(
    <BriefingCardView
      variant="tasks"
      nowMs={NOW}
      briefing={briefing({
        completed_total: 200,
        completed: [
          { initiative_id: 'in-1', initiative: 'Website', count: 150, by_level: { machine_peer: 140, peer: 10 }, tier2: 3 },
          { initiative_id: 'in-2', initiative: 'Mobile', count: 50, by_level: { machine_peer: 50 }, tier2: 0 },
        ],
      })}
    />
  )
  expect(html).toContain('Nothing needs you.')
  expect(html).toContain('HQ verified and completed 200')
  expect(html).toContain('150</span> in Website')
  expect(html).toContain('140 on machine checks and an independent review')
  expect(html).toContain('3 tier 2')
  expect(html).not.toContain('WRK-')
})

test('what needs the owner comes first and is named; past the threshold it becomes one offer', () => {
  const some = renderToString(
    <BriefingCardView variant="tasks" nowMs={NOW} briefing={briefing({ needs_you_total: 2, needs_you: [waitingItem(1), waitingItem(2)] })} />
  )
  expect(some).toContain('Needs you (2)')
  expect(some).toContain('WRK-001')
  expect(some).toContain('held for you')
  expect(some.indexOf('Needs you (2)')).toBeLessThan(some.indexOf('Audit') === -1 ? Infinity : some.indexOf('Audit'))

  const many = renderToString(
    <BriefingCardView
      variant="tasks"
      nowMs={NOW}
      onPlanSweep={noop}
      briefing={briefing({
        needs_you_total: 47,
        needs_you: [],
        bulk_offer: { count: 40, text: '40 of 47 were submitted before verification was enforced.', command: 'hq verify sweep plan' },
      })}
    />
  )
  expect(many).toContain('47 need you')
  expect(many).toContain('40 of 47 were submitted before verification was enforced.')
  expect(many).toContain('Plan a sweep')
  expect(many).not.toContain('WRK-')
})

test('a veto window names the task, counts down and offers the veto; shadow mode is called what it is', () => {
  const html = renderToString(
    <BriefingCardView
      variant="tasks"
      nowMs={NOW}
      onVeto={noop}
      briefing={briefing({
        mode: 'shadow',
        veto_windows: [{ task_id: 'tk-9', display_id: 'WRK-009', title: 'Run the migration', release_at: NOW / 1000 + 5 * 3600 + 20 * 60 }],
        reworked_total: 5,
        reworked: [{ task_id: 'tk-3', display_id: 'WRK-003', title: 'Fix the guard', returns: 1, last_finding: 'a required check failed', status: 'to_do' }],
        open_audits: 2,
      })}
    />
  )
  expect(html).toContain('WRK-009')
  expect(html).toContain('completes in 5h 20m')
  expect(html).toContain('Veto')
  expect(html).toContain('only watching')
  expect(html).toContain('Failed and sent back without you (5)')
  expect(html).toContain('a required check failed')
  expect(html).toContain('2 completed tasks are waiting for your agree or not.')
})

const sweepDetail = (over: Partial<SweepDetail['sweep']> = {}, rest: Partial<SweepDetail> = {}): SweepDetail => ({
  sweep: {
    id: 'sw-1',
    scope: { kind: 'all' },
    status: 'planned',
    plan: {
      total: 5,
      buckets: { passed: 2, failed: 1, no_artifact: 1, needs_you: 1 },
      would_wait_on_you: 1,
      estimate: { reviewer_runs: 2, usd: 0.1, minutes: 1, basis: 'an assumed $0.05 a run: no reviewer runs to learn from yet', reviewer_enabled: true, monthly_ceiling_usd: 20, spent_this_month_usd: 1 },
    },
    note: '',
    created_at: '2026-10-22 01:00:00',
    updated_at: '2026-10-22 01:00:00',
    applied_at: null,
    undone_at: null,
    ...over,
  },
  progress: { total: 5, buckets: { passed: 2, failed: 1, no_artifact: 1, needs_you: 1 }, states: { planned: 5 } },
  items: [
    { sweep_id: 'sw-1', task_id: 'tk-1', display_id: 'WRK-001', title: 'Ship it', tier: 1, bucket: 'passed', state: 'planned', finding: 'all checks green', detail: '', status_before: 'ready_for_review', applied_status: null },
    { sweep_id: 'sw-1', task_id: 'tk-2', display_id: 'WRK-002', title: 'Fix it', tier: 1, bucket: 'failed', state: 'planned', finding: 'a required check failed', detail: '', status_before: 'ready_for_review', applied_status: null },
  ],
  apply_preview: null,
  ...rest,
})

const body = (detail: SweepDetail, over: Partial<Parameters<typeof SweepBodyView>[0]> = {}) =>
  renderToString(<SweepBodyView detail={detail} busy={false} error={null} confirmed={false} onConfirm={noop} onAction={noop} undone={null} {...over} />)

test('a plan shows the groups with what apply does to each, the estimate, and only Run and Cancel', () => {
  const html = body(sweepDetail())
  expect(html).toContain('Plan ready')
  expect(html).toContain('2</span> passed')
  expect(html).toContain('Completed, with the evidence level recorded')
  expect(html).toContain('Back to to_do with what to link; never completed')
  expect(html).toContain('2 reviewer runs, about $0.10 and 1 minute')
  expect(html).toContain('1 would wait on you')
  expect(html).toContain('Run it')
  expect(html).toContain('Cancel')
  expect(html).not.toContain('Apply')
  expect(html).not.toContain('Undo')
  // the items inside a group are there to be opened, with their findings
  expect(html).toContain('WRK-002')
  expect(html).toContain('a required check failed')
})

test('while it runs there is progress and pause, and applying waits for the confirmation it needs', () => {
  const running = body(sweepDetail({ status: 'running' }, { progress: { total: 10, buckets: {}, states: { planned: 3, verdict: 7 } } }))
  expect(running).toContain('Verifying')
  expect(running).toContain('7 of 10 verified')
  expect(running).toContain('aria-valuenow="70"')
  expect(running).toContain('Pause')
  expect(running).not.toContain('Run it')

  const preview = { will_move: 40, will_keep: 3, confirm_over: 25, needs_confirmation: true }
  const ready = body(sweepDetail({ status: 'ready' }, { apply_preview: preview }))
  expect(ready).toContain('Applying moves 40 tasks and leaves 3 where they are.')
  expect(ready).toContain('I have read the groups and want it applied.')
  expect(ready).toContain('Apply (40)')
  // not ticked, so Apply is disabled; ticked, it is not
  expect(ready).toMatch(/<button[^>]*\sdisabled=""[^>]*>(?:(?!<\/button>).)*Apply \(40\)/)
  const ticked = body(sweepDetail({ status: 'ready' }, { apply_preview: preview }), { confirmed: true })
  expect(ticked).not.toMatch(/<button[^>]*\sdisabled=""[^>]*>(?:(?!<\/button>).)*Apply \(40\)/)

  const small = body(sweepDetail({ status: 'ready' }, { apply_preview: { ...preview, will_move: 5, needs_confirmation: false } }))
  expect(small).not.toContain('I have read the groups')
})

test('an applied sweep offers undo and reports what undo left alone', () => {
  const html = body(sweepDetail({ status: 'applied' }), {
    undone: { restored: 12, left_alone: [{ task: 'WRK-004', why: 'it is in_progress now and was changed after the sweep' }] },
  })
  expect(html).toContain('Undo')
  expect(html).toContain('Restored 12 tasks to review.')
  expect(html).toContain('Left alone: WRK-004, it is in_progress now')
})

test('the scope picker only lets a choice start when it has what it needs', () => {
  const picker = (choice: 'all' | 'initiative' | 'since' | 'selected', over = {}) =>
    renderToString(
      <ScopePickerView
        inputs={{ initiativeId: null, selectedIds: [], since: '' }}
        choice={choice}
        onChoice={noop}
        onSince={noop}
        busy={false}
        error={null}
        recent={[]}
        onStart={noop}
        onOpen={noop}
        {...over}
      />
    )
  expect(picker('all')).not.toMatch(/<button[^>]*\sdisabled=""[^>]*>(?:(?!<\/button>).)*Make the plan/)
  expect(picker('initiative')).toMatch(/<button[^>]*\sdisabled=""[^>]*>(?:(?!<\/button>).)*Make the plan/)
  expect(picker('initiative')).toContain('Pick an initiative in the sidebar first.')
  expect(picker('selected')).toContain('Tick some tasks with Select first.')
  expect(picker('since')).toContain('Give a date as YYYY-MM-DD.')
  expect(picker('all')).toContain('Nothing moves until you read the groups and apply.')
  const withRecent = picker('all', {
    recent: [{ id: 'sw-0', scope: { kind: 'all' }, status: 'applied', plan: { total: 30 }, note: '', created_at: '2026-10-20 08:00:00', updated_at: '', applied_at: null, undone_at: null }],
  })
  expect(withRecent).toContain('Earlier sweeps')
  expect(withRecent).toContain('applied · 30 tasks')
})

const card = (over: Partial<AuditCard> = {}): AuditCard => ({
  audit: { id: 'au-1', task_id: 'tk-1', tier: 1, source: 'pull', verifier_config: 'premium', selected_at: '2026-10-22 01:00:00' },
  task: { id: 'tk-1', display_id: 'WRK-001', title: 'Reject closed jobs', status: 'complete' },
  verdict: {
    verdict: 'pass',
    level: 'machine_peer',
    tier: 1,
    artifact: 'pr:example/repo#7',
    verifier: 'verify:abc',
    independence: 'different_model',
    reasons: 'ok',
    criteria: [
      { text: 'closed jobs refuse applications', result: 'pass', note: 'the diff adds the check' },
      { text: 'tests cover it', result: 'unclear' },
    ],
    created_at: '2026-10-22 00:30:00',
  },
  observations: [
    { id: 'o1', kind: 'contract', fact: 'the task states 2 acceptance criteria', ok: null },
    { id: 'o2', kind: 'pr_state', fact: 'example/repo#7 is merged into main', ok: true },
    { id: 'o3', kind: 'required_checks', fact: 'a required check is still pending', ok: false },
  ],
  links: [{ kind: 'pr', ref: 'example/repo#7' }],
  ...over,
})

const cardHtml = (over: Partial<Parameters<typeof AuditCardView>[0]> = {}) =>
  renderToString(
    <AuditCardView card={card()} busy={false} disagreeing={false} note="" onNote={noop} onAgree={noop} onDisagree={noop} onCancelDisagree={noop} onSkip={noop} {...over} />
  )

test('an audit card shows the task, how it was decided and each check, with one tap to answer', () => {
  const html = cardHtml()
  expect(html).toContain('WRK-001')
  expect(html).toContain('Completed on machine checks and an independent review, tier 1')
  expect(html).toContain('different model')
  expect(html).toContain('1 of 2 criteria met')
  expect(html).toContain('the diff adds the check')
  expect(html).toContain('example/repo#7 is merged into main')
  expect(html).toContain('a required check is still pending')
  expect(html).not.toContain('the task states 2 acceptance criteria')
  expect(html).toContain('Agree')
  expect(html).toContain('Disagree')
  expect(html).toContain('aria-label="Skip"')
  expect(html).toContain('Linked: pr example/repo#7')
})

test('disagreeing asks what is wrong and says the task is reopened', () => {
  const html = cardHtml({ disagreeing: true, note: 'the test is empty' })
  expect(html).toContain('What is wrong?')
  expect(html).toContain('the test is empty')
  expect(html).toContain('Reopen it')
  expect(html).not.toContain('>Agree<')
})

test('the stack shows where you are, the running estimate with its sample size, and Audit 10 more', () => {
  const stack: AuditStack = {
    drawn: 0,
    cards: [card(), card({ audit: { ...card().audit, id: 'au-2' } })],
    calibration: calibration({ tiers: [{ tier: 1, audits: 12, disagreements: 1, disagreement_upper: 0.3, escapes: 0, state: 'insufficient', why: [] }] }),
  }
  const props = { stack, index: 0, busy: false, error: null, disagreeing: false, note: '', onNote: noop, onAgree: noop, onDisagree: noop, onCancelDisagree: noop, onSkip: noop, onMore: noop }
  const html = renderToString(<AuditStackView {...props} />)
  expect(html).toContain('1 of 2')
  expect(html).toContain('1 of 12 audits disagreed. Too few to say how often HQ is wrong')
  expect(html).toContain('Audit 10 more')
  const empty = renderToString(<AuditStackView {...props} stack={{ ...stack, cards: [] }} />)
  expect(empty).toContain('Nothing left to answer.')
  expect(empty).toContain('Audit 10 more')
})

const taskVerification = (over: Partial<TaskVerification> = {}): TaskVerification => ({
  verdicts: [
    {
      id: 'vd-2',
      verdict: 'fail',
      level: 'none',
      tier: 1,
      artifact: 'pr:example/repo#7',
      verifier: 'verify:b',
      independence: 'none',
      reasons: 'a required check failed',
      criteria: [{ text: 'tests pass', result: 'fail' }],
      applied: false,
      waiting_for: 'worker',
      created_at: '2026-10-22 02:00:00',
      observations: [{ id: 'o1', kind: 'required_checks', fact: 'check test failed on abc123', ok: false }],
    },
    {
      id: 'vd-1',
      verdict: 'inconclusive',
      level: 'none',
      tier: 1,
      artifact: 'none',
      verifier: 'verify:a',
      independence: 'none',
      reasons: 'nothing to inspect',
      criteria: [],
      applied: false,
      waiting_for: 'worker',
      created_at: '2026-10-21 02:00:00',
      observations: [],
    },
  ],
  waiting_for: { who: 'worker', kind: 'returned', reason: 'a required check failed' },
  hold: null,
  second_opinion: null,
  ...over,
})

const verification = (data: TaskVerification, over: Partial<Parameters<typeof TaskVerificationView>[0]> = {}) =>
  renderToString(
    <TaskVerificationView data={data} taskStatus="to_do" nowMs={NOW} busy={false} confirmingComplete={false} onSecondOpinion={noop} onVeto={noop} onCompleteAnyway={noop} onReturnToDo={noop} {...over} />
  )

test('the drawer section shows who the task waits on, the latest verdict with its evidence, and the older ones folded', () => {
  const html = verification(taskVerification())
  expect(html).toContain('With the worker.')
  expect(html).toContain('a required check failed')
  expect(html).toContain('check test failed on abc123')
  expect(html).toContain('0 of 1 criterion met')
  expect(html).toContain('1 earlier verdict')
  expect(html).toContain('Second opinion')
  expect(html).toContain('Complete anyway')
  // already in to_do: no pointless "return"
  expect(html).not.toContain('Return to to_do')
  expect(verification(taskVerification(), { taskStatus: 'ready_for_review' })).toContain('Return to to_do')
})

test('completing anyway is a two step action and a completed task offers neither override', () => {
  expect(verification(taskVerification(), { confirmingComplete: true })).toContain('Tap again to complete it')
  const done = verification(taskVerification({ waiting_for: null }), { taskStatus: 'complete' })
  expect(done).not.toContain('Complete anyway')
  expect(done).toContain('Return to to_do')
})

test('a tier 2 window shows the countdown and the veto, and a pending second opinion cannot be asked twice', () => {
  const html = verification(
    taskVerification({
      hold: { release_at: NOW / 1000 + 3 * 3600 },
      waiting_for: { who: 'owner', kind: 'veto_window', reason: 'tier 2: it completes soon' },
      second_opinion: { task_id: 'tk-1', display_id: 'WRK-001', requested_at: '2026-10-22 03:00:00', state: 'pending', detail: '', answered_at: null },
    }),
    { taskStatus: 'ready_for_review' }
  )
  expect(html).toContain('Needs you.')
  expect(html).toContain('it completes in 3h 0m unless you veto it.')
  expect(html).toContain('Veto')
  expect(html).toContain('Second opinion asked')
  expect(html).toContain('asked, waiting for another verifier.')
  const disagreed = verification(
    taskVerification({ second_opinion: { task_id: 'tk-1', display_id: 'WRK-001', requested_at: '', state: 'disagreed', detail: 'no test found', answered_at: '2026-10-22 04:00:00' } })
  )
  expect(disagreed).toContain('it disagreed. no test found')
})

test('a task that was never verified says so', () => {
  const html = verification(taskVerification({ verdicts: [], waiting_for: null }))
  expect(html).toContain('Not verified yet.')
  expect(html).not.toContain('>Second opinion<')
})
