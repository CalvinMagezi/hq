import { createFileRoute } from '@tanstack/react-router'
import { MemoryPage } from '~/routes/settings'

export const Route = createFileRoute('/settings/memory')({
  component: MemoryPage,
})
