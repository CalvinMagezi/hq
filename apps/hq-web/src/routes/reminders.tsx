import { createFileRoute } from '@tanstack/react-router'
import { RemindersPage } from '~/components/reminders/RemindersPage'

// Reached from Settings, an Inbox reminder and chat, never from the bar: it is where HQ's own
// follow-ups are kept (`docs/REMINDERS.md`). A reminder's ping links here with `?id=`.
export const Route = createFileRoute('/reminders')({
  validateSearch: (search: Record<string, unknown>): { id?: string } => (typeof search.id === 'string' && search.id ? { id: search.id } : {}),
  component: RemindersPage,
})
