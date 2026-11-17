import { describe, expect, test } from 'bun:test'
import { liveSentence, liveText, secondsRunning, stopOffered, STOP_AFTER_MS } from './liveLine'

const step = (toolName: string, status: 'running' | 'done', inputArgs?: Record<string, unknown>) => ({ toolName, status, inputArgs })

describe('the live line', () => {
  test('says the step running now, and only that one', () => {
    const line = liveSentence([step('vault_read', 'done', { path: 'Notebooks/A.md' }), step('web_fetch', 'running', { url: 'https://www.example.com/x' })])
    expect(line?.key).toBe('step_web_fetch_site')
    expect(line?.params).toEqual({ site: 'example.com' })
  })
  test('has no sentence between steps or before the first', () => {
    expect(liveSentence([])).toBeNull()
    expect(liveSentence([step('vault_read', 'done')])).toBeNull()
  })
  test('says it in the present tense, naming what it touches', () => {
    expect(liveText(liveSentence([step('vault_read', 'running', { path: 'Notebooks/Shop/Stock rules.md' })]))).toBe('Reading “Stock rules”…')
    expect(liveText(liveSentence([step('web_fetch', 'running', { url: 'https://www.example.com/x' })]))).toBe('Reading a page on example.com…')
    expect(liveText(null)).toBeNull()
  })
  test('counts whole seconds and never goes negative', () => {
    expect(secondsRunning(1_000, 4_900)).toBe(3)
    expect(secondsRunning(5_000, 1_000)).toBe(0)
    expect(secondsRunning(undefined, 1_000)).toBe(0)
  })
  test('offers Stop after ten seconds, not before', () => {
    expect(stopOffered(0, STOP_AFTER_MS - 1)).toBe(false)
    expect(stopOffered(0, STOP_AFTER_MS)).toBe(true)
    expect(stopOffered(undefined, 99_999)).toBe(false)
  })
})
