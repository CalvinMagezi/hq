import { createRouter as createTanStackRouter } from '@tanstack/react-router'
import { routeTree } from './routeTree.gen'
import { adoptLinkToken } from './lib/hqAuth'
import { installStaleBuildRecovery } from './lib/staleBuild'
import { RouteError } from './components/RouteError'

export function getRouter() {
  adoptLinkToken()
  if (typeof window !== 'undefined') installStaleBuildRecovery()
  const router = createTanStackRouter({
    routeTree,
    scrollRestoration: true,
    defaultPreload: 'intent',
    defaultPreloadStaleTime: 30_000,
    defaultStaleTime: 30_000,
    defaultErrorComponent: RouteError,
  })
  return router
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof getRouter>
  }
}
