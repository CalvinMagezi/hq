import { describe, expect, test } from 'bun:test'
import { type Incoming, NEEDS_FOLD_TAG, planPush, type Shown, UPDATES_FOLD_TAG } from './pushFold'

const needs = (id: number): Incoming => ({
  title: 'HQ needs your OK',
  body: 'Open HQ to see what it wants to do.',
  url: '/notifications',
  tag: `hq-inbox-${id}`,
  kind: 'needs',
})
const update = (id: number): Incoming => ({ title: 'HQ answered', body: 'Tap to read.', url: '/chat', tag: `hq-u-${id}`, kind: 'updates' })

/** Applies plans the way the worker does and returns what is showing. */
function run(pushes: Incoming[], at = (i: number) => i * 1000): Shown[] {
  let shown: Shown[] = []
  pushes.forEach((p, i) => {
    const plan = planPush(shown, p, at(i))
    shown = shown.filter((s) => !plan.close.includes(s.tag))
    for (const n of plan.show) {
      shown = shown.filter((s) => s.tag !== n.tag).concat({ tag: n.tag, kind: n.kind, at: at(i), count: n.count })
    }
  })
  return shown
}

describe('planPush', () => {
  test('a sixth Needs you push keeps the newest five and folds one', () => {
    const shown = run([1, 2, 3, 4, 5, 6].map(needs))
    expect(shown.length).toBe(6)
    const fold = shown.find((s) => s.tag === NEEDS_FOLD_TAG)
    expect(fold?.count).toBe(1)
    expect(shown.some((s) => s.tag === 'hq-inbox-1')).toBe(false)
    expect(planPush([], needs(9), 0).show[0].title).toBe('HQ needs your OK')
    const seventh = planPush(shown, needs(7), 9000).show.find((s) => s.tag === NEEDS_FOLD_TAG)
    expect(seventh?.title).toBe('2 more waiting for you')
    expect(seventh?.body).toBe('Open HQ to see them.')
  })

  test('four Updates in an hour show as one', () => {
    const shown = run([1, 2, 3, 4].map(update))
    expect(shown.length).toBe(1)
    expect(shown[0].tag).toBe(UPDATES_FOLD_TAG)
    const plan = planPush(run([1, 2, 3].map(update)), update(4), 4000)
    expect(plan.show[0].title).toBe('4 updates from HQ')
    expect(plan.show[0].body).toBe('Tap to read.')
  })

  test('updates spread over more than an hour do not fold', () => {
    const shown = run([1, 2, 3, 4].map(update), (i) => i * 61 * 60 * 1000)
    expect(shown.some((s) => s.tag === UPDATES_FOLD_TAG)).toBe(false)
  })

  test('a replayed item replaces itself instead of adding', () => {
    const shown = run([1, 2, 3, 4, 5].map(needs))
    expect(planPush(shown, needs(3), 9000)).toEqual({ close: [], show: [needs(3)] })
  })
})
