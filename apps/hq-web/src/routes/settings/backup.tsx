import { createFileRoute } from '@tanstack/react-router'
import { BackupPage } from '~/routes/settings'

export const Route = createFileRoute('/settings/backup')({
  component: BackupPage,
})
