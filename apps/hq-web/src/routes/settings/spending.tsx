import { createFileRoute } from '@tanstack/react-router'
import { SpendingPage } from '~/routes/settings'

export const Route = createFileRoute('/settings/spending')({
  component: SpendingPage,
})
