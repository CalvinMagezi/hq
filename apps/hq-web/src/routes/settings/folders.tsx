import { createFileRoute } from '@tanstack/react-router'
import { FoldersPage } from '~/routes/settings'

export const Route = createFileRoute('/settings/folders')({
  component: FoldersPage,
})
