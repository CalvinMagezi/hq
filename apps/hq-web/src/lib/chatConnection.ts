import type { ConnectionState } from '~/lib/connection'
import { t } from '~/strings'

type Key = Parameters<typeof t>[0]

/** The banner words for a connection state, or null when nothing needs saying. */
export function bannerKey(state: ConnectionState, sharedComputer: boolean): Key | null {
  switch (state) {
    case 'starting':
      return 'chat_banner_starting'
    case 'reconnecting':
      return 'chat_banner_reconnecting'
    case 'unreachable':
      return sharedComputer ? 'chat_banner_shared' : 'chat_banner_unreachable'
    case 'offline':
      return sharedComputer ? 'chat_banner_shared' : 'chat_banner_offline'
    case 'ai_down':
      return 'chat_banner_ai_down'
    default:
      return null
  }
}

/** The failure line for a Stop that reached HQ by neither the socket nor HTTP. */
export function stopFailureKey(state: ConnectionState): Key {
  if (state === 'starting') return 'fail_stop_reply_starting'
  if (state === 'unreachable') return 'fail_stop_reply_unreachable'
  if (state === 'offline') return 'fail_stop_reply_offline'
  return 'fail_stop_reply_no_reply'
}

/** The failure line for a message the socket would not take. */
export function sendFailureKey(state: ConnectionState): Key {
  if (state === 'starting') return 'fail_send_message_starting'
  if (state === 'unreachable') return 'fail_send_message_unreachable'
  if (state === 'offline') return 'fail_send_message_offline'
  return 'fail_send_message_no_reply'
}

/**
 * Stop a reply over the socket and over HTTP, so a socket that dropped does not leave it running.
 * Resolves true when either reached HQ.
 */
export async function stopReply(
  threadId: string,
  sendSocket: (frame: Record<string, unknown>) => boolean,
  stopHttp: (threadId: string) => Promise<void>,
): Promise<boolean> {
  const viaSocket = sendSocket({ type: 'stop', thread_id: threadId })
  if (viaSocket) return true
  try {
    await stopHttp(threadId)
    return true
  } catch {
    return false
  }
}
