import { createFileRoute } from '@tanstack/react-router'
import { GithubPage } from '~/routes/settings'

export const Route = createFileRoute('/settings/github')({
  component: GithubPage,
})
