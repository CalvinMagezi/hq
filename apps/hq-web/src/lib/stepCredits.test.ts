import { describe, expect, test } from 'bun:test'
import { firstUnclaimedStep, formatCredits, parseStepCredit, parseStepCredits, totalCredits } from './stepCredits'

describe('formatCredits', () => {
  test('rounds, marks tiny and unknown', () => {
    expect(formatCredits(12.4)).toBe('~12 credits')
    expect(formatCredits(0.3)).toBe('<1 credit')
    expect(formatCredits(0)).toBe('<1 credit')
    expect(formatCredits(null)).toBe('n/a')
    expect(formatCredits(undefined)).toBe('n/a')
  })
})

describe('totalCredits', () => {
  test('sums known figures and ignores unknown ones', () => {
    expect(totalCredits([{ turn: 1, delta: 3 }, { turn: 2, delta: null }, { turn: 3, delta: 4.5 }])).toBe(7.5)
  })
  test('is null when nothing is known', () => {
    expect(totalCredits([{ turn: 1, delta: null }])).toBeNull()
    expect(totalCredits(undefined)).toBeNull()
  })
})

describe('firstUnclaimedStep', () => {
  const ids = ['a', 'b', 'c']
  test('the first step of a run is its first call', () => {
    expect(firstUnclaimedStep(ids, [])).toBe('a')
  })
  test('later steps start after what the previous credit saw', () => {
    expect(firstUnclaimedStep(ids, [{ turn: 1, delta: 1, stepsSeen: 2 }])).toBe('c')
  })
  test('a step with no tool calls has none', () => {
    expect(firstUnclaimedStep(ids, [{ turn: 1, delta: 1, stepsSeen: 3 }])).toBeUndefined()
  })
})

describe('parsing', () => {
  test('reads a saved entry', () => {
    expect(parseStepCredit({ turn: 2, delta: 5, tool_call_id: 'x', model: 'm' })).toEqual({ turn: 2, delta: 5, toolCallId: 'x' })
  })
  test('a missing delta is unknown, a bad entry is dropped', () => {
    expect(parseStepCredit({ turn: 1 })).toEqual({ turn: 1, delta: null, toolCallId: undefined })
    expect(parseStepCredit({ delta: 1 })).toBeNull()
    expect(parseStepCredits('nope')).toBeUndefined()
    expect(parseStepCredits([{ turn: 1, delta: 2 }, 7])?.length).toBe(1)
  })
})
