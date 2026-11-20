import { createFileRoute } from '@tanstack/react-router'
import { AccessPage } from '~/routes/settings'

export const Route = createFileRoute('/settings/access')({
  component: AccessPage,
})
