import { createFileRoute } from '@tanstack/react-router'
import { ChatView } from '~/components/chat/ChatView'

export const Route = createFileRoute('/chat')({
  component: ChatRoutePage,
})

function ChatRoutePage() {
  return (
    <div className="h-full w-full bg-neutral-950">
      <ChatView active />
    </div>
  )
}
