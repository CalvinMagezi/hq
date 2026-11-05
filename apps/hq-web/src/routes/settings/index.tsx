import { createFileRoute } from '@tanstack/react-router'

// The list pane beside it is the page; the layout sends a wide screen on to Notifications.
export const Route = createFileRoute('/settings/')({
  component: () => null,
})
