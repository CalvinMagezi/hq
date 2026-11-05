import { describe, expect, test } from 'bun:test'
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { t } from '~/strings'
import { isHubResponse } from './hubWindow'
import { monthKey } from './dataUsed'
import { DEFAULT_PUSH_PREFS, subscribeBody } from './webPush'
import {
  LANDING_PATH,
  ageWords,
  backupNeedsLook,
  formatSize,
  hashRedirect,
  liteNotice,
  pickCard,
  settingsGroups,
  type BackupStatus,
} from './settingsGroups'

const src = join(import.meta.dir, '..')
const read = (p: string) => readFileSync(join(src, p), 'utf8')
const HERE = { lite: false, hub: false }
const status = (over: Partial<BackupStatus>): BackupStatus => ({ configured: true, last_ok_at: null, failed: false, overdue: false, ...over })

describe('the four groups (check 1)', () => {
  test('headings come in the phone order, This browser first', () => {
    expect(settingsGroups(HERE).map((g) => t(g.label))).toEqual(['This browser', 'Your accounts', 'How HQ behaves', 'Your data'])
  })

  test('rows are the phone names, and the Claude connector is gone (W34)', () => {
    const rows = settingsGroups(HERE).flatMap((g) => g.rows.map((r) => t(r.label)))
    expect(rows).toEqual([
      'Notifications', 'Time zone', 'Install and version', 'Data used', 'Sign out this browser',
      'AI service and keys', 'Messaging apps', 'Other devices',
      'Spending limits', 'Safety and limits', 'Folders helpers may use',
      'Backup and about',
    ])
    expect(JSON.stringify(settingsGroups(HERE))).not.toMatch(/claude/i)
  })

  test('a wide /settings lands on Notifications', () => {
    expect(LANDING_PATH).toBe('/settings/browser')
  })

  test('Lite keeps This browser and Other devices (H8)', () => {
    const rows = settingsGroups({ lite: true, hub: false }).flatMap((g) => g.rows.map((r) => r.id))
    expect(rows).toEqual(['notifications', 'time', 'version', 'data', 'signout', 'devices'])
  })

  test('the HQ window reads "This window" and has no Install and version (H9, B11)', () => {
    const groups = settingsGroups({ lite: false, hub: true })
    expect(t(groups[0].label)).toBe('This window')
    expect(groups[0].rows.map((r) => r.id)).not.toContain('version')
    expect(t(groups[0].rows.at(-1)!.label)).toBe('Sign out this window')
  })

  test('the hub header is read as written', () => {
    expect(isHubResponse(new Headers({ 'x-hq-window': 'hub' }))).toBe(true)
    expect(isHubResponse(new Headers())).toBe(false)
  })
})

describe('addresses (check 2, WR28)', () => {
  test('old #hash links go to their section', () => {
    expect(hashRedirect('#keys')).toBe('/settings/keys')
    expect(hashRedirect('clock')).toBe('/settings/time')
    expect(hashRedirect('#session-folders')).toBe('/settings/folders')
    expect(hashRedirect('#nothing')).toBeNull()
  })

  test('in Lite, spending opens This browser with its line (check 7)', () => {
    expect(liteNotice('/settings/spending', true)).toBe('lite_no_spending')
    expect(t('lite_no_spending')).toBe('This HQ is HQ Lite. It has no Spending limits.')
    expect(liteNotice('/settings/keys', true)).toBe('lite_only')
    expect(liteNotice('/settings/browser', true)).toBeNull()
    expect(liteNotice('/settings/spending', false)).toBeNull()
  })
})

describe('notifications per browser (check 3)', () => {
  test('a new subscription asks for both kinds and no details', () => {
    expect(DEFAULT_PUSH_PREFS).toEqual({ needs: true, updates: true, details: false })
    expect(subscribeBody({ endpoint: 'https://push.example.com/a' }, DEFAULT_PUSH_PREFS)).toEqual({
      endpoint: 'https://push.example.com/a',
      kinds: ['needs', 'updates'],
      details: false,
    })
  })

  test('each switch writes its own kind, and details travel with them', () => {
    expect(subscribeBody({}, { needs: true, updates: false, details: true })).toEqual({ kinds: ['needs'], details: true })
    expect(subscribeBody({}, { needs: false, updates: false, details: false }).kinds).toEqual([])
  })
})

describe('the card slot (check 4)', () => {
  test('a newer HQ goes first, then notifications, then backups, else none', () => {
    expect(pickCard({ newerRunning: true, notificationsOff: true, backupNeedsLook: true })).toBe('newer')
    expect(pickCard({ newerRunning: false, notificationsOff: true, backupNeedsLook: true })).toBe('notif_off')
    expect(pickCard({ newerRunning: false, notificationsOff: false, backupNeedsLook: true })).toBe('backups')
    expect(pickCard({ newerRunning: false, notificationsOff: false, backupNeedsLook: false })).toBeNull()
  })
})

describe('the shell (check 5)', () => {
  test('the header has no clock and the rail has no Usage', () => {
    expect(existsSync(join(src, 'components/HeaderClock.tsx'))).toBe(false)
    expect(read('routes/__root.tsx')).not.toContain('HeaderClock')
    expect(read('components/AppRail.tsx')).not.toMatch(/Usage|\/usage/)
  })

  test('the Inbox has no push button and the chat header no bell', () => {
    expect(existsSync(join(src, 'components/notifications/PushToggle.tsx'))).toBe(false)
    expect(read('routes/notifications.tsx')).not.toContain('PushToggle')
    expect(read('components/chat/ChatHeader.tsx')).not.toMatch(/replyAlerts|BellOff/)
  })

  test('Settings keeps no line about the claude.ai connector (W34)', () => {
    expect(read('routes/settings.tsx')).not.toMatch(/connector/i)
  })
})

describe('data and backup (check 6)', () => {
  test('Data used reads "{N} MB this month in this browser"', () => {
    expect(t('data_used_row', { size: formatSize(120 * 1024 * 1024) })).toBe('120 MB this month in this browser')
    expect(formatSize(0.4 * 1024 * 1024)).toBe('0.4 MB')
    expect(formatSize(80 * 1024)).toBe('80 KB')
  })

  test('the counter starts over on the 1st, in the display zone', () => {
    const edge = new Date('2026-10-31T22:30:00Z')
    expect(monthKey(edge, 'UTC')).toBe('2026-10')
    expect(monthKey(edge, 'Africa/Nairobi')).toBe('2026-11')
  })

  test('no backup route: grey words and no card', () => {
    expect(backupNeedsLook(null)).toBe(false)
    expect(t('backup_not_reported')).toBe('Backups are set up where HQ runs.')
  })

  test('only failed or overdue turns it amber', () => {
    expect(backupNeedsLook(status({ last_ok_at: '2026-11-05T10:00:00Z' }))).toBe(false)
    expect(backupNeedsLook(status({ overdue: true }))).toBe(true)
    expect(backupNeedsLook(status({ failed: true }))).toBe(true)
    expect(backupNeedsLook(status({ configured: false, failed: true }))).toBe(false)
  })

  test('the overdue card says how old the last backup is', () => {
    const now = new Date('2026-11-05T12:00:00Z').getTime()
    const age = ageWords('2026-11-02T00:00:00Z', now)
    expect(age).toBe('3 d')
    expect(t('card_backups_body', { age })).toBe('Last backup 3 d ago. A new one should be made every 24 hours.')
    expect(ageWords('2026-11-05T09:00:00Z', now)).toBe('3 h')
  })
})

import { trailNotifKey } from './settingsGroups'
import { inInboxTab } from './notificationsApi'

describe('rows say what this browser really does', () => {
  test('the Notifications row says off while this browser receives nothing, whatever the switches', () => {
    expect(trailNotifKey({ needs: true, updates: true }, false)).toBe('trail_notif_none')
    expect(trailNotifKey({ needs: true, updates: true }, true)).toBe('trail_notif_both')
    expect(trailNotifKey({ needs: false, updates: true }, true)).toBe('trail_notif_updates')
  })

  test('Needs you holds only what waits for an answer; updates go to Updates', () => {
    const ask = { state: 'pending', kind: 'action_needed' } as const
    const news = { state: 'pending', kind: 'value_item' } as const
    const done = { state: 'dismissed', kind: 'value_item' } as const
    expect([inInboxTab(ask, 'pending'), inInboxTab(ask, 'all'), inInboxTab(ask, 'resolved')]).toEqual([true, false, false])
    expect([inInboxTab(news, 'pending'), inInboxTab(news, 'all')]).toEqual([false, true])
    expect(inInboxTab(done, 'resolved')).toBe(true)
  })
})
