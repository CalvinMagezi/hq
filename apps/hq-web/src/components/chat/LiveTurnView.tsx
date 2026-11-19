import { t } from '~/strings'
import { useLiveTurn } from '~/store/threadStore'
import { useHelperRunsStore } from '~/store/helperRunsStore'
import { countRunningHelpers, resolveChatHelpers } from '~/lib/subagentHelpers'
import { ChatBubble } from './ChatBubble'
import { StreamErrorBoundary } from './StreamErrorBoundary'

// A fixed timestamp keeps the live bubble from re-rendering its footer on every delta.
const LIVE_TIMESTAMP = 0
const EMPTY_RUNS: never[] = []

/** The reply streaming into one chat, in the same bubble it will be saved as. Renders nothing when idle. */
export function LiveTurnView({ threadId, onStop }: { threadId: string | null | undefined; onStop?: () => void }) {
  const turn = useLiveTurn(threadId)
  const helperRuns = useHelperRunsStore((s) => (threadId ? s.runsByThread[threadId] : undefined)) ?? EMPTY_RUNS
  if (!turn) return null
  const runningHelpers = countRunningHelpers(resolveChatHelpers(helperRuns, turn.toolSteps))
  return (
    <>
      <StreamErrorBoundary plainText={turn.content} resetKey={turn.content.length}>
        <ChatBubble
          live
          startedAt={turn.startedAt}
          runningHelpers={runningHelpers}
          onStop={onStop}
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
        <p role="status" className="-mt-2 mb-3 px-3 text-xs text-amber-400">
          {t('chat_reply_skipped')}
        </p>
      )}
    </>
  )
}
