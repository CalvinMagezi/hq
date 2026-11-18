import { createFileRoute } from '@tanstack/react-router'
import { ChatSettings } from '~/components/settings/ChatSettings'

export const Route = createFileRoute('/settings/chat')({
  component: ChatSettings,
})
