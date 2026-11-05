import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { load } from '../../../../scripts/strings/build'
import type { WatchedSession, HarnessSession } from '~/lib/sessionsApi'
import { SessionInspector } from '~/components/sessions/SessionInspector'
import { SessionsPanel } from '~/components/chat/SessionsPanel'

const WORDS_PER_SENTENCE = 15
const RETIRED = /\b(agents?|sessions?|harness|sandbox|provider|scope|native|workbench)\b|\bThis PC\b|Chat apps|Inbox & Approvals|Return to to_do|\bto_do\b|\bclaude-code\b|ago ago|\bMark all read\b|View Details/i

/** What is wrong with the text a person reads, as a list; empty when it is plain. */
function wordProblems(text: string, allowThisPc = false): string[] {
  const problems: string[] = []
  if (text.includes(';')) problems.push('a semicolon')
  for (const sentence of text.split(/\n|(?<=[.!?])\s+/)) {
    if (sentence.trim().split(/\s+/).length > WORDS_PER_SENTENCE) problems.push(`over ${WORDS_PER_SENTENCE} words: ${sentence}`)
  }
  const retired = (allowThisPc ? text.replace(/\bThis PC\b/g, '') : text).match(RETIRED)
  if (retired) problems.push(`the retired word "${retired[0]}"`)
  return problems
}

const textOf = (html: string) =>
  html
    .replace(/ (?:aria-label|title|placeholder)="([^"]*)"/g, ' >$1< ')
    .replace(/<[^>]*>/g, '\n')
    .replace(/&amp;/g, '&')
    .replace(/&#x27;/g, "'")
    .replace(/[ \t]+/g, ' ')
    .replace(/\s*\n\s*/g, '\n')
    .trim()

const sqliteUtc = (ms: number) => new Date(ms).toISOString().slice(0, 19).replace('T', ' ')

const watched = (over: Partial<WatchedSession> = {}): WatchedSession => ({
  id: 'hs-1',
  harness: 'claude-code',
  label: 'shop-site',
  host: 'native',
  cwd: '/work/shop-site',
  status: 'running',
  agent_status: 'blocked',
  last_seen_at: sqliteUtc(Date.now() - 60_000),
  drive: false,
  mode: 'observe',
  goal: null,
  done_criteria: null,
  drive_blocked_by: [],
  drive_off_reason: null,
  nudges_sent: 0,
  pending_wake: null,
  last_driven_at: null,
  created_at: sqliteUtc(Date.now() - 14 * 60_000),
  task: null,
  ...over,
})

const harness = (over: Partial<HarnessSession> = {}): HarnessSession => ({
  ...watched(),
  agent_name: 'a',
  owner_thread: null,
  archived: false,
  ...over,
})

const src = join(import.meta.dir, '..')
const sources = () => Array.from(new Bun.Glob('{components,routes,lib}/**/*.{ts,tsx}').scanSync(src)).filter((p) => !p.includes('.test.'))

describe('the words a person reads', () => {
  test('the old strings are gone from the source', () => {
    const was = [
      'Inbox & Approvals',
      'relay notifications',
      'Search proposals, skills',
      'Resolved History',
      'All Activity',
      'View Details & Diff',
      'Return to to_do',
      'waiting for your agree or not',
      'This device was signed out',
      'New agent',
      'Connect a model',
      'Chat apps',
      'Agent HQ Vault',
      'approved ${clockTime',
      '} ago</Row>',
    ]
    const hits = sources().flatMap((p) => {
      const text = readFileSync(join(src, p), 'utf8')
      const stopAll = /['">]Stop all['"<]/.test(text) ? ['Stop all'] : []
      return [...was.filter((w) => text.includes(w)), ...stopAll].map((w) => `${p}: ${w}`)
    })
    expect(hits).toEqual([])
  })

  test('the Inbox, Helpers and briefing words come from the catalogue', () => {
    const cat = load()
    expect(cat.inbox_title).toBe('Inbox')
    expect([cat.inbox_tab_needs, cat.inbox_tab_updates, cat.inbox_tab_done]).toEqual(['Needs you', 'Updates', 'Done'])
    expect(cat.inbox_search).toBe('Search the Inbox')
    expect(cat.place_helpers).toBe('Helpers')
    expect(cat.row_messaging).toBe('Messaging apps')
    expect(cat.briefing_audits_waiting).toEqual({ one: '1 finished task waits for your look.'.replace('1', '{n}'), other: '{n} finished tasks wait for your look.' })
    expect(cat.push_not_subscribed).toBe('This browser does not have notifications on.')
  })

  test('the Helpers panel and Details pane show a span once and no internal names', () => {
    const inspector = textOf(renderToStaticMarkup(createElement(SessionInspector, { session: harness() })))
    expect(inspector).toContain('Started · 14 min ago')
    expect(inspector).toContain('Last seen · 1 min ago')
    expect(inspector).not.toContain('ago ago')
    expect(inspector.match(/14 min ago/g)?.length).toBe(1)
  })

  test('the Watching panel names a blocked Claude Code in words', () => {
    const html = textOf(renderToStaticMarkup(createElement(SessionsPanel, { sessions: [watched()], busy: new Set<string>(), onDrive: () => {}, onUnwatch: () => {} })))
    expect(html).toContain('Needs your OK')
    expect(html).toContain('Claude Code')
    expect(html).toContain('Let HQ say yes to its requests')
    expect(html).not.toMatch(/blocked|claude-code|native|Drive/)
  })

  test('every rendered place is plain: no long sentence, semicolon or retired word', () => {
    const fixtures = [
      textOf(renderToStaticMarkup(createElement(SessionInspector, { session: harness() }))),
      textOf(renderToStaticMarkup(createElement(SessionsPanel, { sessions: [watched(), watched({ id: 'hs-2', agent_status: 'working' })], busy: new Set<string>(), onDrive: () => {}, onUnwatch: () => {} }))),
    ]
    for (const text of fixtures) expect(wordProblems(text)).toEqual([])
  })

  test('a fixture with "This PC is offline" outside the HQ Hub tab fails the check', () => {
    expect(wordProblems('This PC is offline')).toEqual(['the retired word "This PC"'])
    expect(wordProblems('This PC is offline', true)).toEqual([])
    expect(wordProblems('One; two')).toEqual(['a semicolon'])
    expect(wordProblems('a b c d e f g h i j k l m n o p.')).toHaveLength(1)
  })

  test('the sentences this brief added are plain', () => {
    const keys = ['inbox_', 'helpers_', 'helper_', 'time_', 'approved_when', 'briefing_', 'signin_replaced', 'push_not_subscribed', 'setup_title', 'connect_']
    const cat = load()
    const bad = Object.entries(cat)
      .filter(([k]) => keys.some((p) => k.startsWith(p)))
      .flatMap(([k, v]) => Object.values(typeof v === 'string' ? { v } : v).flatMap((s) => wordProblems(s).map((p) => `${k}: ${p}`)))
    expect(bad).toEqual([])
  })
})
