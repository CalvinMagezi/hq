import { createFileRoute } from '@tanstack/react-router'
import { DataUsedPage } from '~/components/settings/BrowserSections'

export const Route = createFileRoute('/settings/data')({
  component: DataUsedPage,
})
