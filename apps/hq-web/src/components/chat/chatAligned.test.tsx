import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { ChatRow, rowLine } from './ChatList'
import { ChatBubble } from './ChatBubble'

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
