import type { ConnectionState } from '~/lib/connection'
import { helloUpstreamGone } from '~/lib/hello'
import { knownInHub } from '~/lib/hubWindow'
import { t } from '~/strings'

type Key = Parameters<typeof t>[0]

export type FailureCause = 'starting' | 'unreachable' | 'offline' | 'no_reply'

/** Why a request failed, from what the shell knows: only a state that names a cause gets its own words. */
export function causeOf(state: ConnectionState): FailureCause {
  if (state === 'starting' || state === 'unreachable' || state === 'offline') return state
  return 'no_reply'
}

/** The chat archive failure line for a connection state. */
export function archiveChatKey(state: ConnectionState): Key {
  return `fail_archive_chat_${causeOf(state)}` as Key
}

/**
 * A failure line that exists for two causes: HQ out of reach, or any other. `unreachable` is taken
 * from the shell's state, so a drop between the request and the answer still reads as a drop.
 */
export function unreachableOr(state: ConnectionState, unreachable: Key, otherwise: string, params: Record<string, string> = {}): string {
  return causeOf(state) === 'unreachable' ? t(unreachable as never, params as never) : otherwise
}

/** Why actions are off while HQ is out of reach: the HQ window says HQ stopped, a browser says it can't be reached. */
export function offWhyUnreachable(inHub: boolean = knownInHub(), upstreamGone: boolean = helloUpstreamGone()): string {
  return t(inHub && upstreamGone ? 'off_hub_stopped' : 'off_until_reached')
}
