import { describe, expect, test, beforeEach } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { Glob } from 'bun'
import { readFileSync } from 'node:fs'
import { t } from '~/strings'
import { HqHttpError } from '~/lib/hqAuth'
import { clearableUpdateIds, type NotificationItem } from '~/lib/notificationsApi'
import {
  UNDO_WINDOW_MS,
  pressUndo,
  toastBottom,
  toastUndo,
  undoWindowOpen,
  useToasts,
} from '~/lib/toast'
import { ApprovalCard, allowLabel, quoteArgs } from './ApprovalCard'
import { Dialog, dismissOutcome, fitsSideBySide, redLabelProblem } from './Dialog'
import { ToastView, toastPlacement } from './Toast'

const noop = () => {}

function dialog(viewportWidth: number, keepLabel: string, actLabel: string) {
  return renderToStaticMarkup(
    createElement(Dialog, { kind: 'red', title: 'Stop it?', body: 'Body', keepLabel, actLabel, onKeep: noop, onAct: noop, viewportWidth }),
  )
}

describe('Dialog layout (02 check 1)', () => {
  test('the action sits right of the safe button when both fit', () => {
    const html = dialog(768, 'Keep it', 'Stop it')
    expect(html).toContain('data-layout="side"')
    expect(html.indexOf('Keep it')).toBeLessThan(html.indexOf('Stop it</button>'))
  })

  test('the action drops below the safe button when they do not fit, both full width', () => {
    const html = dialog(320, 'Keep them running', 'Stop every helper')
    expect(html).toContain('data-layout="stacked"')
    expect(html).toContain('flex-direction:column')
    expect(html.indexOf('Keep them running')).toBeLessThan(html.indexOf('Stop every helper</button>'))
  })

  test('the fit rule changes with the width', () => {
    expect(fitsSideBySide(390, 'Keep', 'Stop')).toBe(true)
    expect(fitsSideBySide(390, 'Keep them running', 'Stop every helper')).toBe(false)
  })

  test('a red action is the red button; an ask action is the green one', () => {
    expect(dialog(768, 'Keep it', 'Stop it')).toContain('a-btn danger')
    const ask = renderToStaticMarkup(
      createElement(Dialog, { kind: 'ask', title: 'Mark 3 tasks done?', body: 'b', keepLabel: 'Keep', actLabel: 'Mark 3 tasks done', onKeep: noop, onAct: noop, viewportWidth: 768 }),
    )
    expect(ask).not.toContain('danger')
  })

  test('the dialog is a native <dialog> with its title and body named', () => {
    const html = dialog(768, 'Keep it', 'Stop it')
    expect(html.startsWith('<dialog')).toBe(true)
    expect(html).toContain('aria-labelledby')
    expect(html).toContain('aria-describedby')
  })
})

describe('Red dialog labels (02 check 3)', () => {
  const reds: [string, string][] = [
    [t('stop_helper_title', { helper: 'Ada' }), t('stop_helper_act', { helper: 'Ada' })],
    [t('stop_all_title'), t('stop_all_act')],
    [t('sign_out_device_title', { device: 'Phone' }), t('sign_out_device_act')],
    [t('sign_out_all_title'), t('sign_out_all_act')],
    [t('signout_browser_title'), t('row_sign_out_browser')],
    [t('disconnect_app_title', { app: 'Telegram' }), t('disconnect_app_act', { app: 'Telegram' })],
    [t('unlink_account_title', { app: 'Telegram' }), t('unlink_account_act')],
    [t('revoke_folder_title', { folder: 'repo' }), t('revoke_folder_act')],
    [t('delete_task_title'), t('delete_task_act')],
  ]
  test('every red action starts with the title verb and is never a bare Yes', () => {
    for (const [title, act] of reds) expect(redLabelProblem(title, act)).toBeNull()
  })
  test('the old labels are refused', () => {
    expect(redLabelProblem('Stop it?', 'Yes, stop it')).not.toBeNull()
    expect(redLabelProblem('Stop it?', 'OK')).not.toBeNull()
    expect(redLabelProblem('Stop it?', 'Continue')).not.toBeNull()
  })
})

describe('No window.confirm (02 check 4)', () => {
  test('nothing under src calls it', () => {
    const needle = ['window', 'confirm('].join('.')
    const bare = ['confirm', '('].join('')
    const hits: string[] = []
    for (const file of new Glob('src/**/*.{ts,tsx}').scanSync('.')) {
      if (file.endsWith('.test.ts') || file.endsWith('.test.tsx')) continue
      const text = readFileSync(file, 'utf8')
      if (text.includes(needle) || new RegExp(`(^|[^.\\w])${bare.replace('(', '\\(')}`).test(text)) hits.push(file)
    }
    expect(hits).toEqual([])
  })
})

describe('Leaving a dialog never acts (02 check 5)', () => {
  test('Esc, the scrim and Back all mean keep', () => {
    for (const way of ['esc', 'scrim', 'back'] as const) expect(dismissOutcome(way, false)).toBe('keep')
  })
  test('nothing dismisses it while it is working', () => {
    for (const way of ['esc', 'scrim', 'back'] as const) expect(dismissOutcome(way, true)).toBe('ignore')
  })
})

describe('Approval card (02 checks 2 and 6)', () => {
  const del = { tool: 'vault_delete', args: { path: 'Projects/Old plan.md' }, loses: true, verb: 'delete' }
  const card = (q: typeof del | { tool: string; args: Record<string, unknown>; loses: boolean; verb: null }) =>
    renderToStaticMarkup(
      createElement(ApprovalCard, {
        quoteTitle: t('approval_quote_chat'),
        tool: q.tool,
        args: q.args,
        inWords: 'HQ wants to tidy up.',
        loses: q.loses,
        verb: q.verb,
        onDecline: noop,
        onAllow: noop,
      }),
    )

  test('quotes the tool and the arguments above "In HQ\'s words"', () => {
    const html = card(del)
    expect(quoteArgs(del.args)).toBe('{"path": "Projects/Old plan.md"}')
    expect(html).toContain('vault_delete')
    expect(html).toContain('{&quot;path&quot;: &quot;Projects/Old plan.md&quot;}')
    expect(html.indexOf('vault_delete')).toBeLessThan(html.indexOf("In HQ&#x27;s words"))
  })

  test('a request that loses something is "Allow delete" and never green', () => {
    const html = card(del)
    expect(allowLabel(true, 'delete')).toBe('Allow delete')
    expect(html).toContain('Allow delete')
    expect(html).toContain('a-btn danger')
    expect(html).not.toContain('class="a-btn"')
  })

  test('a plain request is "Allow once" in green, after Decline', () => {
    const html = card({ tool: 'vault_write', args: { path: 'a.md' }, loses: false, verb: null })
    expect(html).toContain('Allow once')
    expect(html).toContain('class="a-btn"')
    expect(html.indexOf('Decline')).toBeLessThan(html.indexOf('Allow once'))
  })
})

describe('Undo toasts (03 checks 5 and 6)', () => {
  beforeEach(() => useToasts.setState({ toasts: [] }))
  const latest = () => useToasts.getState().toasts.at(-1)!

  test('Chat archived offers Undo, and Undo calls the restore and says restored', async () => {
    let called = 0
    const id = toastUndo(t('undo_chat_archived'), async () => void called++, t('restored_chat'))
    expect(latest().text).toBe('Chat archived')
    expect(await pressUndo(id)).toBe('restored')
    expect(called).toBe(1)
    expect(latest().text).toBe('Chat restored')
    expect(latest().undo).toBeUndefined()
  })

  test('a 409 from the restore says it is too late', async () => {
    const id = toastUndo('Chat archived', async () => Promise.reject(new HqHttpError('changed', 409)), 'Chat restored')
    expect(await pressUndo(id)).toBe('too_late')
    expect(latest().text).toBe(t('fail_undo_too_late'))
    expect(latest().kind).toBe('alert')
  })

  test('after 10 minutes Undo does not call the restore at all', async () => {
    let called = 0
    const id = toastUndo('Task archived', async () => void called++, 'Task restored')
    const at = latest().at
    expect(undoWindowOpen(at, at + UNDO_WINDOW_MS - 1)).toBe(true)
    expect(await pressUndo(id, at + UNDO_WINDOW_MS)).toBe('too_late')
    expect(called).toBe(0)
  })

  test('the Undo button is named after what it undoes', () => {
    const html = renderToStaticMarkup(
      createElement(ToastView, { toast: { id: 1, text: 'Chat archived', kind: 'status', undo: async () => {}, at: 0, late: false }, wide: false, bottom: 72, onUndo: noop, onClose: noop }),
    )
    expect(html).toContain('aria-label="Undo: Chat archived"')
    expect(html).toContain('a-snack-act')
  })

  test('an alert is read out loud', () => {
    const html = renderToStaticMarkup(
      createElement(ToastView, { toast: { id: 1, text: 'No', kind: 'alert', at: 0, late: true }, wide: true, bottom: 24, onUndo: noop, onClose: noop }),
    )
    expect(html).toContain('role="alert"')
  })

  test('the toast sits full width on a phone and bottom right from 768', () => {
    expect(toastPlacement(390).wide).toBe(false)
    expect(toastPlacement(768).wide).toBe(true)
  })

  test('the toast never covers the focused field', () => {
    const height = 800
    const field = { top: 700, bottom: 740, left: 0, right: 300 }
    const bottom = toastBottom(72, 48, height, field)
    const toastTop = height - bottom - 48
    expect(toastTop + 48 <= field.top || toastTop >= field.bottom).toBe(true)
    expect(toastBottom(72, 48, height, { top: 100, bottom: 140, left: 0, right: 1 })).toBe(72)
  })
})

describe('Clear all updates (03)', () => {
  const item = (id: string, kind: NotificationItem['kind'], state: NotificationItem['state']) => ({ id, kind, state }) as NotificationItem
  test('only pending updates are cleared; a Needs-you request never is', () => {
    const ids = clearableUpdateIds([item('a', 'value_item', 'pending'), item('b', 'action_needed', 'pending'), item('c', 'value_item', 'dismissed')])
    expect(ids).toEqual(['a'])
  })
})
