import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryView, memoryMeta, type MemoryViewProps } from './MemoryView'
import type { MemoryItem, MemoryList } from '~/lib/memoryApi'
import { t } from '~/strings'

const NOW = Date.UTC(2026, 10, 27, 12)

const item = (over: Partial<MemoryItem> = {}): MemoryItem => ({
  id: 1,
  summary: 'Acme delivers on Thursdays',
  entities: [],
  topics: [],
  importance: 0.5,
  created_at: new Date(NOW - 5 * 86_400_000).toISOString(),
  status: 'active',
  origin: 'owner',
  provenance: 'model_inferred',
  pinned: false,
  used: 0,
  thread_id: null,
  harness: 'web',
  ...over,
})

const list = (items: MemoryItem[], recall: MemoryList['recall'] = 'shadow'): MemoryList => ({
  counts: { total: items.length, active: items.length, review: 0, out_of_date: 0, unattributed: 0, pinned: 0 },
  items,
  recall,
})

const noop = () => {}
const props = (over: Partial<MemoryViewProps> = {}): MemoryViewProps => ({
  list: list([item()]),
  filter: 'all',
  query: '',
  canChange: true,
  busyId: null,
  error: null,
  forgotten: null,
  onFilter: noop,
  onQuery: noop,
  onSave: async () => true,
  onPin: noop,
  onStanding: noop,
  onForget: async () => true,
  nowMs: NOW,
  ...over,
})

const html = (over: Partial<MemoryViewProps> = {}) => renderToStaticMarkup(createElement(MemoryView, props(over)))

describe('the page', () => {
  test('it says what it is, how memories reach a chat, and how many there are', () => {
    const out = html({ list: list([item(), item({ id: 2 })], 'query') })
    expect(out).toContain(t('memory_intro'))
    expect(out).toContain(t('memory_mode_query'))
    expect(out).toContain(t('memory_counts', { total: 2, active: 2, review: 0, outdated: 0 }))
    expect(out).toContain('Acme delivers on Thursdays')
  })

  test('there is a labelled search box and the five filters, with the chosen one pressed', () => {
    const out = html({ filter: 'review' })
    expect(out).toContain(`for="memory-search"`)
    expect(out).toContain(t('memory_search_label'))
    for (const key of ['memory_filter_all', 'memory_filter_review', 'memory_filter_outdated', 'memory_filter_unattributed', 'memory_filter_pinned'] as const) {
      expect(out).toContain(t(key))
    }
    expect(out.match(/aria-pressed="true"/g)?.length).toBe(1)
  })

  test('a search shows no filter as chosen', () => {
    expect(html({ filter: 'review', query: 'acme' }).match(/aria-pressed="true"/g)).toBeNull()
  })

  test('an empty list says why: nothing yet, or nothing that matches', () => {
    expect(html({ list: list([]) })).toContain(t('memory_empty'))
    expect(html({ list: list([]), filter: 'pinned' })).toContain(t('memory_empty_filter'))
    expect(html({ list: list([]), query: 'zebra' })).toContain(t('memory_empty_filter'))
  })

  test('being offline says so and turns every change off', () => {
    const out = html({ canChange: false, initial: { openId: 1 } })
    expect(out).toContain(t('memory_offline'))
    const buttons = out.match(/<button[^>]*>[^<]*<\/button>/g) ?? []
    const actions = buttons.filter((b) => [t('memory_edit'), t('memory_pin'), t('memory_forget')].some((w) => b.includes(`>${w}<`)))
    expect(actions.length).toBe(3)
    for (const b of actions) expect(b).toContain('disabled')
  })

  test('an error is announced', () => {
    expect(html({ error: t('memory_action_failed') })).toContain('role="alert"')
  })
})

describe('one memory', () => {
  test('its line says how long ago, what is unusual and how often it was used', () => {
    const line = memoryMeta(item({ pinned: true, used: 3, provenance: 'owner_edited' }), NOW)
    expect(line).toContain(t('memory_badge_pinned'))
    expect(line).toContain(t('memory_badge_edited'))
    expect(line).toContain('Used 3 times')
    expect(memoryMeta(item({ used: 1 }), NOW)).toContain('Used 1 time')
    expect(memoryMeta(item({ used: 3 }), NOW)).toContain('Used 3 times')
  })

  test('a note from a shared channel says why it is held back', () => {
    const out = html({ list: list([item({ origin: 'unknown_channel', status: 'review' })]) })
    expect(out).toContain(t('memory_badge_shared'))
    expect(out).toContain(t('memory_hint_shared'))
  })

  test('options stay closed until asked for, then offer what fits the note', () => {
    expect(html()).not.toContain('data-testid="memory-options"')

    const ordinary = html({ initial: { openId: 1 } })
    for (const key of ['memory_edit', 'memory_pin', 'memory_mark_outdated', 'memory_forget'] as const) expect(ordinary).toContain(t(key))
    expect(ordinary).not.toContain(`>${t('memory_keep')}<`)

    const held = html({ list: list([item({ status: 'review', origin: 'unknown_channel' })]), initial: { openId: 1 } })
    expect(held).toContain(`>${t('memory_keep')}<`)
    expect(held).not.toContain(t('memory_mark_outdated'))

    const old = html({ list: list([item({ status: 'superseded' })]), initial: { openId: 1 } })
    expect(old).toContain(t('memory_use_again'))

    const pinned = html({ list: list([item({ pinned: true })]), initial: { openId: 1 } })
    expect(pinned).toContain(`>${t('memory_unpin')}<`)
  })

  test('editing shows the text in a labelled box with Save and Cancel', () => {
    const out = html({ initial: { editingId: 1 } })
    expect(out).toContain(`for="memory-edit-1"`)
    expect(out).toContain(t('memory_edit_label'))
    expect(out).toContain(t('memory_save'))
    expect(out).toContain(t('memory_cancel'))
  })

  test('a note being changed is marked busy', () => {
    expect(html({ busyId: 1 })).toContain('aria-busy="true"')
  })
})

describe('forgetting', () => {
  test('the question names what is lost and the red button starts with the title verb', () => {
    const out = html({ initial: { confirmId: 1 } })
    expect(out).toContain(t('memory_forget_title'))
    expect(out).toContain(t('memory_forget_body'))
    expect(out).toContain(t('memory_forget_act'))
    expect(t('memory_forget_title').split(' ')[0]).toBe(t('memory_forget_act').split(' ')[0])
  })

  test('afterwards it says what HQ could not reach', () => {
    const out = html({ forgotten: { stillThere: ['memory_still_chat', 'memory_still_backups'] } })
    expect(out).toContain(t('memory_forgotten'))
    expect(out).toContain(t('memory_still_chat'))
    expect(out).toContain(t('memory_still_backups'))
    expect(html({ forgotten: { stillThere: [] } })).not.toContain(t('memory_still_there'))
  })
})
