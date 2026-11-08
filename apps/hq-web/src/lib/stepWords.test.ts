import { describe, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { createElement } from 'react'
import { stepLine, stepLines, stepText } from './stepWords'
import { ToolCallBlock } from '~/components/chat/ToolCallBlock'
import type { ToolStep } from '~/store/threadStore'

const say = (name: string, args?: Record<string, unknown>) => stepText(stepLine(name, args))

describe('what a reply did, in plain words', () => {
  test('a step names what it touched when its arguments say', () => {
    expect(say('web_search', { query: 'UK public holidays November 2026' })).toBe('Searched the web for “UK public holidays November 2026”')
    expect(say('web_fetch', { url: 'https://www.timeanddate.com/holidays/uk/' })).toBe('Read a page on timeanddate.com')
    expect(say('vault_read', { path: 'Notebooks/Projects/Mobile-First-Roadmap.md' })).toBe('Read “Mobile-First-Roadmap”')
    expect(say('task_create', { title: 'Call the bank' })).toBe('Added the task “Call the bank”')
  })

  test('without the name it still says what kind of thing it did', () => {
    expect(say('web_search')).toBe('Searched the web')
    expect(say('web_fetch', { url: 'not a url' })).toBe('Read a web page')
    expect(say('task_list')).toBe('Checked your tasks')
    expect(say('bash', { command: 'ls' })).toBe('Ran a command')
  })

  test('a tool no table names falls back by family, then to its own name, never "Did a step"', () => {
    expect(say('vault_something_new')).toBe('Worked with your notes')
    expect(say('harness_session_peek')).toBe('Checked on a helper')
    expect(say('system_clock')).toBe('Used system clock')
  })

  test("the gateway's hq_call is read as the tool it calls", () => {
    expect(say('hq_call', { tool: 'web_search', args: { query: 'rain in Kampala' } })).toBe('Searched the web for “rain in Kampala”')
  })

  test('long names are cut to fit a phone', () => {
    const line = say('web_search', { query: 'a'.repeat(200) })
    expect(line.length).toBeLessThan(90)
    expect(line).toContain('…')
  })

  test('a run of the same line is said once, different lines all stay', () => {
    const lines = stepLines([
      { toolName: 'task_list' },
      { toolName: 'task_list' },
      { toolName: 'web_search', inputArgs: { query: 'a' } },
      { toolName: 'web_search', inputArgs: { query: 'b' } },
      { toolName: 'task_list' },
    ]).map(stepText)
    expect(lines).toEqual(['Checked your tasks', 'Searched the web for “a”', 'Searched the web for “b”', 'Checked your tasks'])
  })
})

describe('the What I did block', () => {
  const step = (id: string, toolName: string, inputArgs?: Record<string, unknown>): ToolStep => ({
    toolCallId: id, toolName, status: 'done', startedAt: 0, durationMs: 1200, progressMessages: [], inputArgs,
  })

  test('folded, it counts the lines and shows no tool names', () => {
    const html = renderToStaticMarkup(
      createElement(ToolCallBlock, { steps: [step('a', 'web_search', { query: 'x' }), step('b', 'web_fetch', { url: 'https://example.com' }), step('c', 'web_fetch', { url: 'https://example.com' })] }),
    )
    expect(html).toContain('What I did · 2 steps')
    expect(html).not.toContain('web_search')
    expect(html).not.toContain('web_fetch')
  })
})
