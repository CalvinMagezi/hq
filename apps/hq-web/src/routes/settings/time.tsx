import { createFileRoute } from '@tanstack/react-router'
import { TimeZonePage } from '~/components/settings/TimeZonePage'

export const Route = createFileRoute('/settings/time')({
  component: TimeZonePage,
})
