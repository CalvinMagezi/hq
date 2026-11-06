import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { ChatRow } from './ChatList'

const read = (p: string) => readFileSync(join(import.meta.dir, p), 'utf8')

// web-matches-phone B1: Chat on the shared panes (AlignedChat).
describe('Chat on the panes', () => {
  test('the chats are the list pane, the chat the detail, its details the side pane, and a sheet below 1024 px', () => {
    const view = read('ChatView.tsx')
    expect(view).toContain('<Panes')
    expect(view).toMatch(/list=\{chatList\(true\)\}[\s\S]*side=\{side\}/)
    expect(view).toContain('placement="bottom"')
    expect(view).toContain('a-ib a-back')
    expect(view).toContain('a-ib sm a-side-toggle')
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
