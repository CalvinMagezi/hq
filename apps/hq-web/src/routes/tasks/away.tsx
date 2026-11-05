import { createFileRoute } from '@tanstack/react-router'

// Opens the Tasks page; the parent route draws it. The "While you were away" view is the Tasks place's own work.
export const Route = createFileRoute('/tasks/away')({})
