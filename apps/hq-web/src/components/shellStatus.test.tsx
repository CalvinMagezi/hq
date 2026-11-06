import { describe, expect, test } from 'bun:test'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { connectionState, type ConnectionState, type UseConnectionResult } from '~/lib/connection'
import { BalanceChip, HeaderTitle, placeTitle } from './StatusLine'
import { savedLine, stripShown } from './SavedStrip'
import { helperState } from './sessions/SessionRow'
import { inboxNeedsYou } from './AppRail'
import { showsSectionError } from './vault/HomeSection'
import { lateLabel } from './tasks/TaskCard'

const root = join(import.meta.dir, '..', '..')
const read = (p: string) => readFileSync(join(root, p), 'utf8')
const STATES: ConnectionState[] = ['signed_out', 'offline', 'unreachable', 'starting', 'reconnecting', 'ai_down', 'connected']

describe('header in the seven states', () => {
  for (const state of STATES) {
    test(`${state}: one status line, no clock, chip only when connected`, () => {
      const conn = { state, words: `words of ${state}`, dot: 'warn' } as unknown as UseConnectionResult
      const html = renderToStaticMarkup(createElement(HeaderTitle, { title: placeTitle('/tasks'), connection: conn }))
      expect(html).toContain(`words of ${state}`)
      expect(html.match(/a-status/g)?.length).toBe(1)
      expect(html).not.toMatch(/\d\d:\d\d/)
      const chip = renderToStaticMarkup(createElement(BalanceChip, { state, label: '$4' }))
      expect(chip.includes('a-bal')).toBe(state === 'connected')
    })
  }
  test('connectionState still names every state in words', () => {
    expect(typeof connectionState).toBe('function')
  })
})

describe('badge and strip', () => {
  test('rail and bar badge is amber and Workbench has none', () => {
    expect(inboxNeedsYou(2, 3)).toBe(5)
    for (const f of ['src/components/AppRail.tsx', 'src/components/BottomNav.tsx', 'src/components/Places.tsx']) expect(read(f)).not.toMatch(/workbenchBadge|workbench[A-Za-z]*Count/i)
    expect(read('src/styles/aligned-ui.css')).toMatch(/\.a-badge\{background:var\(--warn\)/)
  })
  test('strip shows in every state but connected and signed_out', () => {
    expect(STATES.filter(stripShown)).toEqual(['offline', 'unreachable', 'starting', 'reconnecting', 'ai_down'])
  })
  test('saved line', () => {
    const now = new Date(2026, 10, 3, 15, 0).getTime()
    expect(savedLine(null, now)).not.toContain('undefined')
    expect(savedLine(now - 3600_000, now, 'en')).not.toBe(savedLine(now - 3 * 86400_000, now, 'en'))
  })
  test('helper states', () => {
    expect(helperState({ status: 'running', agent_status: 'blocked', alive: true })).toBe('needs')
    expect(helperState({ status: 'running', alive: false })).toBe('lost')
    expect(helperState({ status: 'running', reachable: false })).toBe('offline')
  })
  test('section errors only while connected', () => {
    expect(showsSectionError(false, 'error')).toBe(false)
    expect(showsSectionError(true, 'error')).toBe(true)
  })
  test('late label', () => {
    expect(lateLabel({ due_date: '2020-01-01', status: 'to_do' })?.late).toBe(true)
    expect(lateLabel({ due_date: '2020-01-01', status: 'complete' })).toBeNull()
  })
})

describe('colours and sizes', () => {
  test('dim text and no font under 12px in app.css', () => {
    const css = read('app.css')
    expect(css).toMatch(/--text-dim:\s*var\(--ink-muted\)/)
    expect(read('src/styles/aligned-ui.css')).toMatch(/--ink-muted:#D0D2D3/i)
    expect(css).not.toMatch(/font-size:\s*(?:[0-9]|10|11)px/)
  })
  test('narrow phone: Search moves to the menu, status wraps', () => {
    expect(read('src/styles/aligned-ui.css')).toMatch(/@container app \(max-width:359px\)\{\.a-top-more\{display:inline-flex\}/)
    expect(read('src/styles/aligned-ui.css')).toMatch(/\.a-status\{white-space:normal\}/)
  })
  test('no rose or emerald status colour in the shell-owned files', () => {
    const files = [
      'src/components/StatusLine.tsx', 'src/components/SavedStrip.tsx', 'src/components/AppRail.tsx', 'src/components/BottomNav.tsx',
      'src/components/tasks/TaskCard.tsx', 'src/components/tasks/TaskDetailDrawer.tsx', 'src/components/tasks/verification/TaskVerification.tsx',
      'src/components/tasks/verification/AuditSheet.tsx', 'src/components/chat/SessionsPanel.tsx', 'src/components/sessions/SessionRow.tsx',
      'src/components/sessions/BlockedCallout.tsx', 'src/components/sessions/SessionInspector.tsx', 'src/components/SignalCard.tsx',
    ]
    for (const f of files) expect([f, read(f).match(/(rose|emerald|green|red)-\d{3}|text-\[(?:8|9|10|11)px\]/g)]).toEqual([f, null])
  })
  test('no Tailscale word in shell components', () => {
    for (const f of readdirSync(join(root, 'src/components')).filter((n) => n.endsWith('.tsx') && !n.endsWith('.test.tsx'))) {
      expect([f, /tailscale/i.test(read(`src/components/${f}`))]).toEqual([f, false])
    }
  })
})
