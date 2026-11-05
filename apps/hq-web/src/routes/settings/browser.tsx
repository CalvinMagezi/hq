import { createFileRoute } from '@tanstack/react-router'
import { NotificationsPage } from '~/components/settings/NotificationsPage'

export const Route = createFileRoute('/settings/browser')({
  component: NotificationsPage,
})
