import { expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router'
import { NotFoundPage, Route as AppRoot } from './__root'

test('the app root registers the unknown-route page', () => {
  expect(AppRoot.options.notFoundComponent).toBe(NotFoundPage)
})

test('an unknown address shows the page inside the layout, so the sidebar stays', async () => {
  const root = createRootRoute({
    component: () => (
      <div>
        <nav data-testid="sidebar">sidebar</nav>
        <Outlet />
      </div>
    ),
    notFoundComponent: AppRoot.options.notFoundComponent,
  })
  const chat = createRoute({ getParentRoute: () => root, path: '/chat', component: () => <p>chat</p> })
  const router = createRouter({
    routeTree: root.addChildren([chat]),
    history: createMemoryHistory({ initialEntries: ['/nope'] }),
  })
  await router.load()
  const html = renderToStaticMarkup(<RouterProvider router={router} />)
  expect(html).toContain('sidebar')
  expect(html).toContain('This page is not here')
  expect(html).toContain('Back')
  expect(html).toContain('Go to chat')
  expect(html).not.toContain('>chat</p>')
})
