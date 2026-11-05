import { describe, expect, test } from 'bun:test'
import { agoText, describe as view, kindOf, lockdownWho, stopIsRisky, type Report } from './thisComputer'

const base: Report = {
  state: 'stopped',
  start: null,
  exited_at: null,
  you_stopped_at: null,
  last_run: null,
  usual_secs: null,
  program: { path: '/usr/local/bin/hq', version: 'hq 0.9.2' },
  serves_hub: false,
  hub_web: false,
  free_bytes: null,
  almost_full: false,
  lockdown: null,
  helpers: { reachable: true, working: [], needs_you: [] },
}
const starting = (over: object): Report => ({
  ...base,
  state: 'starting',
  start: { step: 3, steps: 4, elapsed_secs: 12, stalled: false, restarting: false, ...over },
})

describe('This computer tab', () => {
  test('off says HQ Hub starts nothing by itself', () => {
    const v = view(base, false, null)
    expect(v.title).toBe('HQ is off')
    expect(v.lines[0]).toContain('starts nothing by itself')
  })
  test('starting shows the step, the seconds and the usual time', () => {
    const v = view({ ...starting({}), usual_secs: 20 }, false, null)
    expect(v.title).toBe('Starting HQ · 12 s')
    expect(v.lines[0]).toBe('Step 3 of 4 · 12 s · Opening your notes and tasks')
    expect(v.lines[1]).toBe('Usually about 20 s on this computer.')
  })
  test('a start that stands still is slow and offers a restart', () => {
    expect(kindOf(starting({ stalled: true }), false)).toBe('slow')
  })
  test('a restart says Restarting HQ', () => {
    expect(view(starting({ restarting: true }), false, null).title).toBe('Restarting HQ')
  })
  test('in hub mode the card talks about helpers for the phone', () => {
    const off = view({ ...base, serves_hub: true }, false, 'Pixel 8')
    expect(off.title).toBe('Helpers here, for HQ on your phone')
    expect(off.lines[0]).toBe('HQ runs on your phone, Pixel 8. This computer runs only the helpers it starts.')
    const on = view({ ...base, state: 'running', serves_hub: true, helpers: { reachable: true, working: ['a', 'b'], needs_you: [] } }, false, null)
    expect(on.lines[0]).toBe('2 helpers are working here for HQ on your phone.')
  })
  test('a lockdown wins over everything but a missing program and names who did it', () => {
    const r = { ...base, state: 'running' as const, lockdown: { reason: 'lost it', source: 'operator', at: 0 } }
    const v = view(r, false, 'Pixel 8')
    expect(v.kind).toBe('locked')
    expect(v.lines[0]).toContain('from Pixel 8')
    expect(v.lines[1]).toBe('Reason given: lost it')
    expect(lockdownWho('cli', null)).toBe('this computer')
    expect(kindOf({ ...r, program: null }, false)).toBe('not_found')
  })
  test('a held port blocks Open HQ and says it waits', () => {
    const v = view({ ...base, serves_hub: true }, true, null)
    expect(v.kind).toBe('port_held')
    expect(v.openWhy).toBe("Open HQ waits until HQ Hub holds the port itself.")
  })
  test('stopping is risky only while helpers are working', () => {
    expect(stopIsRisky(base)).toBe(false)
    expect(stopIsRisky({ ...base, helpers: { reachable: true, working: ['a'], needs_you: [] } })).toBe(true)
  })
  test('a host that ended by itself says when and the exit code', () => {
    const v = view({ ...base, state: 'exited', code: 1, exited_at: 1000, last_run: { started_at: 0, ended_at: 1000 } }, false, null)
    expect(v.kind).toBe('stopped')
    expect(v.lines[1]).toContain('Exit code 1')
  })
})

describe('words that were written but not shown', () => {
  test('a stalled start says where it stopped', () => {
    const lines = view(starting({ stalled: true }), false, null).lines
    expect(lines.at(-1)).toBe('Stopped at: ' + lines[0].split(' · ').at(-1))
  })
  test('the off card has its subtitle', () => {
    expect(view(base, false, null).lines[1]).toBe('HQ on this computer')
  })
  test('ago uses the shared relative-time words', () => {
    expect(agoText(20)).toBe('now')
    expect(agoText(5 * 60)).toBe('5 min ago')
    expect(agoText(3 * 3600)).toBe('3 h ago')
    expect(agoText(86400)).toBe('yesterday')
    expect(agoText(3 * 86400)).toBe('3 days ago')
  })
})
