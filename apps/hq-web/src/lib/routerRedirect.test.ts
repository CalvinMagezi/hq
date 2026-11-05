import { expect, test } from 'bun:test'
import { createMemoryHistory, createRouter } from '@tanstack/react-router'
import { routeTree } from '~/routeTree.gen'

// Loading an old address makes the root's beforeLoad throw a redirect; its Location header is where it goes.
const land = async (url: string) => {
  const router = createRouter({ routeTree, history: createMemoryHistory({ initialEntries: [url] }) })
  await router.load()
  const r = router.state.redirect as Response | undefined
  return r ? r.headers.get('location') : url
}

test('old addresses land on the new ones through the real router', async () => {
  expect(await land('/vault/Projects/HQ/Plan.md')).toBe('/notes/Projects/HQ/Plan.md')
  expect(await land('/sessions?id=h-31')).toBe('/helpers/h-31')
  expect(await land('/notifications')).toBe('/inbox')
  expect(await land('/chat?thread=abc#x')).toBe('/chat/abc#x')
  expect(await land('/tasks?task=T-12')).toBe('/tasks/T-12')
  expect(await land('/usage')).toBe('/settings/spending')
  expect(await land('/')).toBe('/chat')
})

test('current addresses are not redirected', async () => {
  expect(await land('/notes/Projects/HQ/Plan.md')).toBe('/notes/Projects/HQ/Plan.md')
  expect(await land('/inbox')).toBe('/inbox')
})
