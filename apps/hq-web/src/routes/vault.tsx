import { createFileRoute } from '@tanstack/react-router'

// Old address: the root's beforeLoad (legacyRedirect) sends it to its new home before this draws.
export const Route = createFileRoute('/vault')({})
