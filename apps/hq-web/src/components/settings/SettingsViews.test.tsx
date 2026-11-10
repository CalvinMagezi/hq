import { expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { HqSettings, ProviderKeyRow } from '~/lib/settingsApi'
import { t } from '~/strings'
import { BackupView, FoldersView, SafetyView } from './SettingsViews'
import { KeysView, keyRowAction } from './ProviderKeysSection'

const html = (el: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(el)

const settings = {
  safety: { bash_sandbox: 'best_effort', bash_network: true, skills_write_approval: false, web_allowed_origins: [] },
  limits: { chat_turn_timeout_secs: 600, turn_ack_timeout_secs: 270, background_turn_max_days: 5, background_progress_secs: null },
  integrations: { remote_mcp: [], searxng_host: null, web_search_native: true, disk_watchdog_enabled: true },
  agent_host: { default_host: 'local', hosts: [], sandbox_mode: 'process', sandbox_profile: 'open', sandbox_hide_credentials: true, sandbox_extra_domains: 2, idle_reap_hours: 4, drive_new_watches: true, driver_checkin_minutes: 5, driver_nudge_budget: 6, driver_no_progress_limit: 3 },
} as unknown as HqSettings

test('the safe box reads On · open and passwords hidden Yes, with no config words', () => {
  const out = html(createElement(SafetyView, { s: settings, tab: 'helpers', onTab: () => {} }))
  expect(out).toContain('On · open')
  expect(out).toContain('Saved passwords hidden from helpers')
  expect(out).toContain('Yes')
  expect(out).not.toMatch(/credential stores visible|extra allowed sites|open profile/)
})

test('the four safety tabs are read only', () => {
  const out = html(createElement(SafetyView, { s: settings, tab: 'safety', onTab: () => {} }))
  expect(out.match(/role="tab"/g)).toHaveLength(4)
  expect(out).not.toContain('role="switch"')
})

const row = (o: Partial<ProviderKeyRow>): ProviderKeyRow => ({ name: 'X', configured: true, ...o }) as ProviderKeyRow

test('DeepSeek has no Add and an env Anthropic has no button', () => {
  expect(keyRowAction(row({ name: 'DeepSeek', configured: false }), true)).toBe('none')
  expect(keyRowAction(row({ id: 'anthropic', source: 'env' }), true)).toBe('env')
  const out = html(
    createElement(KeysView, { rows: [row({ name: 'Anthropic', id: 'anthropic', source: 'env' } as never)], canEdit: true, reachable: true, editing: null, saved: null, onEdit: () => {}, onSaved: () => {}, onCancel: () => {} }),
  )
  expect(out).toContain('Set where HQ runs. Change it there, then restart HQ.')
  expect(out).not.toContain('<button')
  expect(out).not.toMatch(/last checked|checked \d/i)
})

test('a folder reads "approved <day> for Claude" and Revoke needs HQ', () => {
  const now = Date.UTC(2026, 9, 30, 12)
  const g = { id: 1, host: 'Mac mini', path: '/work/app', requested_by: 'claude', approved_at: Date.UTC(2026, 9, 24, 12) / 1000 }
  const out = html(createElement(FoldersView, { grants: [g], canChange: false, busy: false, onRevoke: () => {}, nowMs: now }))
  expect(out).toMatch(/approved .*24.* for Claude|approved Oct 24 for Claude|approved 24 Oct for Claude/)
  expect(out).toContain('disabled')
  expect(out).toContain(t('settings_folders_revoke_needs').replace("'", '&#x27;'))
})

test('backup reads Last backup 3 h ago, grey unless a backup can be started', () => {
  const ok = new Date(Date.now() - 3 * 3600_000 - 60_000).toISOString()
  const out = html(createElement(BackupView, { status: { configured: true, last_ok_at: ok, failed: false, overdue: false }, canStart: false, version: null }))
  expect(out).toContain('Last backup 3 h ago')
  const late = html(createElement(BackupView, { status: { configured: true, last_ok_at: ok, failed: false, overdue: true }, canStart: false, version: null }))
  expect(late).not.toContain('a-warn-t')
})

test('Disconnect Telegram says it is set up from the start again', () => {
  expect(t('disconnect_app_body', { app: 'Telegram' })).toContain('To use Telegram again, you set it up from the start.')
})

import { SilenceView, pauseLine } from './SilenceSection'
import type { Silences } from '~/lib/notificationsApi'

const quiet = (over: Partial<Silences> = {}): Silences => ({
  pause: { topic: 'all', silenced: false, until: null },
  topics: ['helpers', 'tasks', 'tips', 'upkeep', 'email'].map((topic) => ({ topic, silenced: topic === 'helpers', until: null })),
  ...over,
})
const silenceProps = { busy: false, onPause: () => undefined, onResume: () => undefined, onTopic: () => undefined }

test('Silence lists every topic as a switch that is on unless silenced, and says what always gets through', () => {
  const out = html(createElement(SilenceView, { silences: quiet(), ...silenceProps }))
  expect(out).toContain(t('silence_intro'))
  expect(out.match(/role="switch"/g)?.length).toBe(5)
  expect(out.match(/aria-checked="false"/g)?.length).toBe(1)
  expect(out).toContain(t('silence_pause_hour'))
  expect(out).not.toContain(t('silence_resume'))
  // HQ's own requests are not a topic, so there is nothing to switch off for them.
  expect(out).not.toContain('>HQ<')
})

test('a pause shows when it ends and offers to turn back on', () => {
  expect(pauseLine({ topic: 'all', silenced: false, until: null })).toBeNull()
  expect(pauseLine({ topic: 'all', silenced: true, until: null })).toBe(t('silence_paused'))
  const out = html(createElement(SilenceView, { silences: quiet({ pause: { topic: 'all', silenced: true, until: '2026-11-10T18:00:00Z' } }), ...silenceProps }))
  expect(out).toContain(t('silence_resume'))
  expect(out).not.toContain(t('silence_pause_hour'))
})
