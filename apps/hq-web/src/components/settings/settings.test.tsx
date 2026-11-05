import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { DEFAULT_PUSH_PREFS } from '~/lib/webPush'
import { CardView } from './CardSlot'
import { NotificationsView, type NotificationsMode } from './NotificationsPage'

const view = (mode: NotificationsMode, prefs = DEFAULT_PUSH_PREFS) =>
  renderToStaticMarkup(createElement(NotificationsView, { mode, prefs, busy: false, host: 'hq.example.ts.net', testLine: null, onToggle: () => {}, onTest: () => {} }))

describe('Notifications page', () => {
  test('three real switches named by what they turn on, details off', () => {
    const html = view('on')
    expect(html.match(/role="switch"/g)?.length).toBe(3)
    for (const name of ['Needs you', 'Updates', 'Show details in notifications']) expect(html).toContain(`aria-label="${name}"`)
    expect(html).toMatch(/aria-label="Show details in notifications"[^>]*aria-checked="false"|aria-checked="false"[^>]*aria-label="Show details in notifications"/)
    expect(html).not.toMatch(/Push on|Turn on push/)
  })

  test('a test can be sent only while on', () => {
    expect(view('on')).toContain('Send a test')
    expect(view('off')).not.toContain('Send a test')
  })

  test('blocked for the site names the host and locks the switches', () => {
    const html = view('denied')
    expect(html).toContain('Notifications are blocked for this site')
    expect(html).toContain('hq.example.ts.net')
    expect(html).toContain('disabled')
  })

  test('a shared computer gets no switches', () => {
    const html = view('shared')
    expect(html).toContain('Not on a shared computer')
    expect(html).not.toContain('role="switch"')
  })

  test('the HQ window has no switches and says its notice has one button', () => {
    const html = view('hub')
    expect(html).not.toContain('role="switch"')
    expect(html).toContain('Its only button is Open.')
  })
})

describe('Card slot', () => {
  const card = (c: 'newer' | 'notif_off' | 'backups', last: string | null = null) =>
    renderToStaticMarkup(createElement(CardView, { card: c, backup: { configured: true, last_ok_at: last, failed: false, overdue: true }, onAct: () => {} }))

  test('the three cards use their exact words', () => {
    expect(card('newer')).toContain('A newer HQ is running')
    expect(card('notif_off')).toContain('Questions from helpers wait in the Inbox until you look.')
    expect(card('backups')).toContain('Check your backups')
    expect(card('backups')).toContain('Open Backup and about')
  })
})
