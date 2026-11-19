import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { ChatRow, rowLine } from './ChatList'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider, createMemoryHistory, createRootRoute, createRouter } from '@tanstack/react-router'
import { ChatBubble } from './ChatBubble'
import { WorkPane } from './WorkPane'

const read = (p: string) => readFileSync(join(import.meta.dir, p), 'utf8')

// web-matches-phone B1: Chat on the shared panes (AlignedChat).
describe('Chat on the panes', () => {
  test('the chats are the list pane, the chat the detail, the work pane the side pane, and a sheet below 1024 px', () => {
    const view = read('ChatView.tsx')
    expect(view).toContain('<Panes')
    expect(view).toMatch(/list=\{chatList\(true\)\}[\s\S]*side=\{pane\.open \? side : undefined\}/)
    expect(view).toContain('placement="bottom"')
    expect(view).toContain('a-ib a-back')
    // The pane has its own toggle at every width; the old details button hid itself when docked.
    expect(view).toContain("t('chat_pane_toggle')")
    expect(view).not.toContain('a-side-toggle')
    expect(view).not.toMatch(/emerald|amber-|neutral-9|ThreadSidebar|ChatHeader/)
  })
  test('a chat row is the design row: title, its line, the time or Working, and an archive button named for it', () => {
    const row = (thread: object, active: boolean, working: boolean) =>
      renderToStaticMarkup(createElement(ChatRow, { thread: { status: 'active', createdAt: 0, updatedAt: Date.now(), unreadCount: 0, ...thread } as never, active, working, spoken: false, onOpen() {}, onArchive() {} }))
    const html = row({ threadId: 'a', title: 'Release checklist', lastMessagePreview: 'Watching 1 helper' }, true, false) + row({ threadId: 'b', title: 'Soak test', unreadCount: 2 }, false, true)
    expect(html).toContain('<a class="a-row" href="/chat/a" aria-current="page"><span class="a-row-main"><b>Release checklist</b><span>Watching 1 helper</span>')
    expect(html).toContain('<span class="a-state working"><span class="a-spin" aria-hidden="true"></span>Working</span>')
    expect(html).toContain('class="a-unread"')
    expect(html).toContain('aria-label="Archive Soak test"')
    expect(read('ChatList.tsx')).toContain('<p className="a-grp">')
  })
  test('a chat row shows its last line as plain words, not as markdown', () => {
    const row = (preview: string) =>
      renderToStaticMarkup(createElement(ChatRow, { thread: { threadId: 'p', title: 'Plan', status: 'active', createdAt: 0, updatedAt: Date.now(), unreadCount: 0, lastMessagePreview: preview } as never, active: false, working: false, spoken: false, onOpen() {}, onArchive() {} }))
    const html = row('**Drive switched off for session kan') + row('Created **[Host Fleet and Worksp') + row('Saved the plan at **`Notebooks/P')
    expect(html).toContain('<span>Drive switched off for session kan</span>')
    expect(html).toContain('<span>Created Host Fleet and Worksp</span>')
    expect(html).toContain('<span>Saved the plan at Notebooks/P</span>')
    expect(html).not.toContain('**')
  })
  test('a chat with text the person began and did not send shows that text as a draft', () => {
    const other = rowLine({ draft: 'Remind me to book the', active: false, spoken: false, preview: 'Done.' })
    expect(other).toEqual({ hasDraft: true, line: 'Remind me to book the' })
    // The open chat shows its composer, so its row stays as it was.
    expect(rowLine({ draft: 'Remind me to book the', active: true, spoken: false, preview: 'Done.' })).toEqual({ hasDraft: false, line: 'Done.' })
    // Spaces alone are not a draft.
    expect(rowLine({ draft: '   ', active: false, spoken: false, preview: 'Done.' }).hasDraft).toBe(false)
    expect(rowLine({ draft: '', active: false, spoken: true, preview: '**Heard** you' }).line).toBe('Spoken · Heard you')
  })
  test('the row marks a draft with the word Draft', () => {
    expect(read('ChatList.tsx')).toContain("t('chat_row_draft')")
    expect(read('ChatList.tsx')).toContain('useDraftText(th.threadId)')
  })
  test('a saved reply can be kept as a note or started as a task, with named buttons', () => {
    const bubble = (role: 'user' | 'assistant', handlers: object) =>
      renderToStaticMarkup(createElement(ChatBubble, { message: { id: 'm1', role, content: 'Hello there.', timestamp: 0 }, ...handlers }))
    const reply = bubble('assistant', { onSave() {}, onMakeTask() {} })
    expect(reply).toContain('aria-label="Save to Vault"')
    expect(reply).toContain('aria-label="Make a task"')
    expect(reply).toContain('aria-label="Copy"')
    // No handler, no button: a reply that is still streaming, or a helper's post, is not offered.
    const plain = bubble('assistant', {})
    expect(plain).not.toContain('Save to Vault')
    expect(plain).not.toContain('Make a task')
  })
  test('only HQ\'s own saved replies are offered the two actions', () => {
    const list = read('MessageList.tsx')
    expect(list).toMatch(/onSave=\{onSave && m\.role === 'assistant' && !liveTurn && isSaved\(m\) && !m\.driver/)
    expect(list).toMatch(/onMakeTask=\{onMakeTask && m\.role === 'assistant' && !liveTurn && isSaved\(m\) && !m\.driver/)
  })
  test('the details pane is the design card list, not the old strip', () => {
    const panel = read('SessionsPanel.tsx')
    expect(panel).toContain('className="hq-chat-side"')
    expect(panel).toContain('className="a-sw" role="switch"')
    expect(panel).not.toMatch(/neutral-|max-w-4xl|border-white/)
  })
  test('the thread, messages and composer are the design ones (AlignedChat)', () => {
    expect(read('MessageList.tsx')).toContain('a-wrap a-thread')
    const bubble = read('ChatBubble.tsx')
    expect(bubble).toContain("${isUser ? 'a-u' : 'a-a'}")
    expect(bubble).toContain('className="a-meta"')
    expect(bubble).not.toMatch(/rgba\(|backdropFilter|emerald|<User /)
    const comp = read('ChatComposer.tsx')
    for (const part of ['className="a-comp"', 'className="a-comp-in"', 'className="a-comp-field"', '"a-send stop"', '"a-send"']) expect(comp, part).toContain(part)
    expect(comp).not.toMatch(/emerald|amber-500|className="[^"]*hq-composer|hq-field/)
    expect(read('StartScreen.tsx')).toContain('className="a-mark"')
  })
})

describe('A reply reads result first', () => {
  const steps = [
    { toolCallId: 'a', toolName: 'vault_read', status: 'done' as const, startedAt: 0, progressMessages: [], inputArgs: { path: 'Notebooks/Shop/Stock rules.md' } },
    { toolCallId: 'b', toolName: 'vault_write_note', status: 'done' as const, startedAt: 0, progressMessages: [], inputArgs: { path: 'Notebooks/Daily/Steps.md' } },
  ]
  const html = (live: boolean) =>
    renderToStaticMarkup(
      createElement(
        QueryClientProvider,
        { client: new QueryClient() },
        createElement(ChatBubble, { live, startedAt: 1, message: { id: 'm', role: 'assistant', content: 'I wrote the steps down.', timestamp: 0, toolSteps: steps } }),
      ),
    )

  test('a saved reply shows its words, then the fold, the made card and the source chips', () => {
    const out = html(false)
    const at = (s: string) => out.indexOf(s)
    // The words render through the markdown viewer, so its box marks where they are.
    expect(at('markdown-viewer')).toBeGreaterThan(-1)
    expect(at('What I did · 2 steps')).toBeGreaterThan(at('markdown-viewer'))
    expect(at('hq-made')).toBeGreaterThan(at('What I did'))
    expect(at('hq-sources')).toBeGreaterThan(at('hq-made'))
    expect(out).toContain('Note · saved to Vault › Daily')
    expect(out).toContain('Stock rules')
  })
  test('a streaming reply shows one live line above its words and none of the saved extras', () => {
    const out = html(true)
    expect(out.match(/class="hq-live"/g)).toHaveLength(1)
    expect(out.indexOf('hq-live')).toBeLessThan(out.indexOf('markdown-viewer'))
    for (const gone of ['What I did', 'hq-made', 'hq-sources']) expect(out).not.toContain(gone)
  })
  test('the offer is not in the bubble: it has the one slot above the box', () => {
    expect(read('ChatBubble.tsx')).not.toContain('OfferLine')
    expect(read('ChatView.tsx')).toMatch(/<ChatApprovals[\s\S]*?otherwise=\{slotOffer\}/)
    expect(read('ChatView.tsx')).toContain('!liveTurn && messages.length > 0')
  })
})

describe('the composer', () => {
  test('keeps the Enter rule in a line of hints, with the choice one click away', () => {
    const comp = read('ChatComposer.tsx')
    expect(comp).toContain("'chat_hint_sends'")
    expect(comp).toContain("t('chat_hint_change')")
    expect(comp).not.toContain('type="checkbox"')
  })
  test('hears @ and # and takes a whole reference back with one Backspace', () => {
    const comp = read('ChatComposer.tsx')
    expect(comp).toContain('triggerAt(value, caret)')
    expect(comp).toContain('referenceBefore(value')
    expect(comp).toContain('<TriggerMenu')
  })
})

describe('WorkPane Result and Helpers tabs', () => {
  async function renderPane(tab: 'result' | 'helpers') {
    const view = createElement(
      QueryClientProvider,
      { client: new QueryClient() },
      createElement(WorkPane, {
        pane: { open: true, tab, note: null, source: null },
        onTab: () => {},
        onClose: () => {},
        steps: [],
        running: false,
        onSelectSource: () => {},
        onAsk: () => {},
        onTask: () => {},
        madeNotes: [{ title: 'Report.html', path: 'Notebooks/Report.html' }],
        madeFiles: [{ path: 'Exports/2026-11/Sheet.xlsx', name: 'Sheet.xlsx', mime: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', kind: 'sheet', size: 1024 }],
        chatHelpers: [
          { id: 'h1', name: 'explorer', goal: 'Check Q3 revenue', status: 'working', background: false },
          { id: 'h2', name: 'coder', goal: 'Draft HTML summary', status: 'done', background: true },
        ],
        helpers: createElement('div', null, 'watched'),
      }),
    )
    const router = createRouter({
      routeTree: createRootRoute({ component: () => view }),
      history: createMemoryHistory({ initialEntries: ['/'] }),
    })
    await router.load()
    return renderToStaticMarkup(createElement(RouterProvider, { router }))
  }

  test('Result tab lists notes and files made in the chat instead of Nothing made yet', async () => {
    const html = await renderPane('result')
    expect(html).not.toContain('Nothing made yet')
    expect(html).toContain('Report.html')
    expect(html).toContain('Sheet.xlsx')
  })

  test('Helpers tab lists this chat’s sub-agent helpers with role, goal and status', async () => {
    const html = await renderPane('helpers')
    expect(html).toContain('In this chat')
    expect(html).toContain('explorer')
    expect(html).toContain('Check Q3 revenue')
    expect(html).toContain('Working')
    expect(html).toContain('coder')
    expect(html).toContain('Draft HTML summary')
    expect(html).toContain('Done')
    expect(html).toContain('Sent to run in the background')
  })
})
