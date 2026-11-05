import { expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { SignedOutView } from './SignInPrompt'
import { en } from '~/strings/en.gen'

test('signed-out Inbox names the state and keeps nothing', () => {
  const html = renderToStaticMarkup(<SignedOutView pathname="/inbox" embedded={false} />)
  expect(html).toContain('Signed out of this HQ')
  expect(html).toContain('Nothing from this HQ stays in this browser.')
  expect(html).toContain('min-h-11')
})

test('Settings words for the new pages exist', () => {
  expect(en.settings_not_here_title).toBe('Not on this HQ')
  expect(en.settings_app_pitch).toContain('about five minutes')
  expect(en.ver_updates_value).toBe('By itself')
})
