import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { HelpersAsk } from './HelpersAsk'
import { ChatSettings } from './ChatSettings'
import { ChatTitle } from '../chat/ChatTitle'

// The Chat page asks the window whether the screen is a touch one.
const storage = { getItem: () => null, setItem: () => undefined }
;(globalThis as { window?: unknown }).window ??= { matchMedia: () => ({ matches: false }), localStorage: storage }
;(globalThis as { localStorage?: unknown }).localStorage ??= storage

const draw = (node: ReturnType<typeof createElement>, prefs?: object) => {
  const client = new QueryClient()
  if (prefs) client.setQueryData(['prefs'], prefs)
  return renderToStaticMarkup(createElement(QueryClientProvider, { client }, node))
}

describe('Helpers ask me for', () => {
  test('is the phone words, three choices, with the one HQ holds selected', () => {
    const html = draw(createElement(HelpersAsk), { 'chat.auto_title': true, 'helpers.ask_level': 'loses_work' })
    expect(html).toContain('Helpers ask me for')
    for (const choice of ['Nothing, my agents handle it', 'Only things that lose work', 'Everything']) expect(html).toContain(choice)
    expect(html).toMatch(/aria-checked="true"[^>]*>(?:(?!<\/button>).)*Only things that lose work/s)
    expect(html).not.toContain('cannot change it yet')
  })
  test('cannot be chosen until HQ has said what it holds', () => {
    const html = draw(createElement(HelpersAsk))
    expect(html).toContain('disabled')
    expect(html).toContain('HQ has not said yet')
  })
})

describe('chat titles', () => {
  test('the Chat page has a switch for HQ naming chats, on when HQ says so', () => {
    const html = draw(createElement(ChatSettings), { 'chat.auto_title': true, 'helpers.ask_level': 'nothing' })
    expect(html).toContain('Let HQ name chats')
    expect(html).toMatch(/aria-checked="true"[^>]*aria-labelledby="st-titles-label"/)
  })
  test('a title can be pressed to rename it, but only when the chat exists', () => {
    expect(renderToStaticMarkup(createElement(ChatTitle, { title: 'Shop stock', onRename: () => undefined }))).toContain('hq-title-btn')
    expect(renderToStaticMarkup(createElement(ChatTitle, { title: 'New chat' }))).not.toContain('hq-title-btn')
  })
})
