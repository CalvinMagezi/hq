import { createFileRoute } from '@tanstack/react-router'
import { DevicesPage } from '~/routes/settings'

export const Route = createFileRoute('/settings/devices')({
  component: DevicesPage,
})
