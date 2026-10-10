import { expect, test } from 'bun:test'
import { isInvalidEstimate, parseEstimate } from './taskFields'

test('an estimate is a positive whole number of minutes', () => {
  expect(parseEstimate('90')).toBe(90)
  expect(parseEstimate(' 5 ')).toBe(5)
  for (const bad of ['', '0', '-3', '1.5', 'abc']) expect(parseEstimate(bad)).toBeNull()
})

test('only text that is neither empty nor valid is invalid, so an empty field still clears', () => {
  expect(isInvalidEstimate('')).toBe(false)
  expect(isInvalidEstimate('   ')).toBe(false)
  expect(isInvalidEstimate('45')).toBe(false)
  for (const bad of ['0', '1.5', '-2', 'soon']) expect(isInvalidEstimate(bad)).toBe(true)
})
