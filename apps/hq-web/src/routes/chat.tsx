import { createFileRoute, useParams } from '@tanstack/react-router'
import { ChatView } from '~/components/chat/ChatView'

export const Route = createFileRoute('/chat')({
  validateSearch: (search: Record<string, unknown>): { compose?: '1' } => (search.compose === '1' ? { compose: '1' } : {}),
  component: ChatRoutePage,
})

function ChatRoutePage() {
  const threadId = useParams({ strict: false }).id
  return (
    <div className="h-full w-full">
      <ChatView active threadId={threadId} />
    </div>
  )
}
