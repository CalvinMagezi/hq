import { createFileRoute } from '@tanstack/react-router'
import { KeysPage } from '~/routes/settings'

export const Route = createFileRoute('/settings/keys')({
  component: KeysPage,
})
