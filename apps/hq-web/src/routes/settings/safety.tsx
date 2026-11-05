import { createFileRoute } from '@tanstack/react-router'
import { SafetyPage } from '~/routes/settings'

export const Route = createFileRoute('/settings/safety')({
  component: SafetyPage,
})
