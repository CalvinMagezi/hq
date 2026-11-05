import { createFileRoute } from '@tanstack/react-router'
import { VersionPage } from '~/components/settings/BrowserSections'

export const Route = createFileRoute('/settings/version')({
  component: VersionPage,
})
