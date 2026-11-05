import { describe, expect, test } from 'bun:test'
import { archiveChatKey, offWhyUnreachable, unreachableOr } from './failureWords'
import { savedBannerLine } from './chatConnection'
import { t } from '~/strings'

describe('failure words', () => {
  test('archive chat names the cause', () => {
    expect(archiveChatKey('unreachable')).toBe('fail_archive_chat_unreachable')
    expect(archiveChatKey('offline')).toBe('fail_archive_chat_offline')
    expect(archiveChatKey('starting')).toBe('fail_archive_chat_starting')
    expect(archiveChatKey('connected')).toBe('fail_archive_chat_no_reply')
  })
  test('unreachable fallback', () => {
    expect(unreachableOr('unreachable', 'fail_stop_helper_unreachable', 'x', { helper: 'A' })).toBe(t('fail_stop_helper_unreachable', { helper: 'A' }))
    expect(unreachableOr('connected', 'fail_stop_helper_unreachable', 'x', { helper: 'A' })).toBe('x')
  })
  test('off why in the hub', () => {
    expect(offWhyUnreachable(true, true)).toBe(t('off_hub_stopped'))
    expect(offWhyUnreachable(false, true)).toBe(t('off_until_reached'))
  })
  test('saved banner line', () => {
    expect(savedBannerLine('connected', false, false, 1)).toBeNull()
    expect(savedBannerLine('offline', true, false, 1)).toBeNull()
    expect(savedBannerLine('offline', false, true, 1)).toBe(t('chat_banner_saved_window', { time: new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' }).format(new Date(1)) }))
  })
})
