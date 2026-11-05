import { createFileRoute } from '@tanstack/react-router'
import { MessagingPage } from '~/routes/settings'

export const Route = createFileRoute('/settings/messaging')({
  component: MessagingPage,
})
