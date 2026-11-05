import { createFileRoute, redirect } from '@tanstack/react-router'

// HQ has no lookup from a note's id to its path yet, so the "Copy link" form opens Notes home until it does.
export const Route = createFileRoute('/notes/n/$id')({
  beforeLoad: () => {
    throw redirect({ to: '/notes', replace: true })
  },
})
