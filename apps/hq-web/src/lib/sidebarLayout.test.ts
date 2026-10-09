import { describe, expect, test } from 'bun:test'
import { DEFAULT_SIDEBAR, SIDEBAR_MAX, SIDEBAR_MIN, clampSidebarWidth, parseSidebarLayout } from './sidebarLayout'

describe('clampSidebarWidth', () => {
  test('keeps a width inside the limits', () => {
    expect(clampSidebarWidth(400)).toBe(400)
    expect(clampSidebarWidth(10)).toBe(SIDEBAR_MIN)
    expect(clampSidebarWidth(5000)).toBe(SIDEBAR_MAX)
  })

  test('rounds and rejects non-numbers', () => {
    expect(clampSidebarWidth(400.6)).toBe(401)
    expect(clampSidebarWidth(Number.NaN)).toBe(DEFAULT_SIDEBAR.width)
  })
})

describe('parseSidebarLayout', () => {
  test('reads a saved layout', () => {
    expect(parseSidebarLayout('{"width":450,"collapsed":true}')).toEqual({ width: 450, collapsed: true })
  })

  test('clamps a saved width', () => {
    expect(parseSidebarLayout('{"width":9999}').width).toBe(SIDEBAR_MAX)
  })

  test('falls back to the default for missing or broken values', () => {
    expect(parseSidebarLayout(null)).toEqual(DEFAULT_SIDEBAR)
    expect(parseSidebarLayout('not json')).toEqual(DEFAULT_SIDEBAR)
    expect(parseSidebarLayout('42')).toEqual(DEFAULT_SIDEBAR)
  })
})
