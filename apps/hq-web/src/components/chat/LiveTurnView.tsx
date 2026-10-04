import { useLiveTurn } from '~/store/threadStore'
import { ChatBubble } from './ChatBubble'
import { StreamErrorBoundary } from './StreamErrorBoundary'

// A fixed timestamp keeps the live bubble from re-rendering its footer on every delta.
const LIVE_TIMESTAMP = 0

/** The reply streaming into one chat, in the same bubble it will be saved as. Renders nothing when idle. */
export function LiveTurnView({ threadId }: { threadId: string | null | undefined }) {
  const turn = useLiveTurn(threadId)
  if (!turn) return null
  return (
    <>
      <StreamErrorBoundary plainText={turn.content} resetKey={turn.content.length}>
        <ChatBubble
          live
          message={{
            id: `live-${threadId}`,
            role: 'assistant',
            content: turn.content,
            reasoning: turn.reasoning,
            toolSteps: turn.toolSteps,
            stepCredits: turn.stepCredits,
            timestamp: LIVE_TIMESTAMP,
          }}
        />
      </StreamErrorBoundary>
      {turn.gap && (
        <p role="status" className="-mt-2 mb-3 px-3 text-xs font-mono text-amber-400">
          Some of this reply was skipped on a slow connection. The complete reply replaces it when HQ finishes.
        </p>
      )}
    </>
  )
}
