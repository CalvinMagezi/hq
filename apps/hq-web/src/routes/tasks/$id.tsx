import { createFileRoute } from '@tanstack/react-router'

// The parent route draws the page and reads the id from the address, so this one has no component.
export const Route = createFileRoute('/tasks/$id')({})
