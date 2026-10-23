package example.hq.spike.bridge

/** A ticket this phone shows. [endsAt] is a wall-clock time in milliseconds, so the screen can count down. */
data class OpenTicket(val link: String, val endsAt: Long)

data class WaitingRequest(val joiner: String, val name: String, val endsAt: Long)

sealed interface JoinState {
    data object Idle : JoinState
    data object Connecting : JoinState
    /** Connected: the person types these on the other device. */
    data class ShowDigits(val value: String) : JoinState
    data class Done(val id: String, val name: String) : JoinState
    data class Failed(val why: String) : JoinState
}

/** Everything the pairing screens show about the bridge process. */
data class BridgeState(
    val enabled: Boolean = false,
    val running: Boolean = false,
    val deviceId: String? = null,
    val relay: String? = null,
    val ticket: OpenTicket? = null,
    val request: WaitingRequest? = null,
    /** How the last ticket ended, until a new one opens. */
    val ticketEnd: TicketEnd? = null,
    val join: JoinState = JoinState.Idle,
    val peers: List<BridgePeer>? = null,
    val line: String? = null,
)

enum class TicketEnd { Paired, Declined, TooLate, Expired, Failed }

/** Folds what `hq bridge run` says into what the screens show. Pure, so each step is tested. */
object BridgeReducer {
    fun reduce(s: BridgeState, e: BridgeEvent, now: Long): BridgeState = when (e) {
        is BridgeEvent.Ready -> s.copy(running = true, deviceId = e.id, relay = e.relay)
        is BridgeEvent.Ticket -> s.copy(ticket = OpenTicket(e.link, now + e.expiresInSecs * MS_PER_SEC), request = null, ticketEnd = null)
        is BridgeEvent.Request -> s.copy(request = WaitingRequest(e.joiner, e.name, now + e.windowSecs * MS_PER_SEC))
        is BridgeEvent.Decision -> s.copy(
            request = null,
            ticketEnd = when (e.result) {
                "paired" -> TicketEnd.Paired
                "too_late" -> TicketEnd.TooLate
                else -> TicketEnd.Declined
            },
        )
        is BridgeEvent.TicketClosed -> s.copy(
            ticket = null, request = null,
            // The device can be on the list after "too late" was said; the end of the ticket has the last word.
            ticketEnd = if (e.paired) TicketEnd.Paired else s.ticketEnd ?: TicketEnd.Expired,
        )
        is BridgeEvent.Digits -> s.copy(join = JoinState.ShowDigits(e.value))
        is BridgeEvent.Joined -> s.copy(join = JoinState.Done(e.id, e.name))
        is BridgeEvent.JoinFailed -> s.copy(join = JoinState.Failed(e.why))
        is BridgeEvent.Peers -> s.copy(peers = e.peers)
        is BridgeEvent.Revoked -> s.copy(peers = s.peers?.filterNot { it.id == e.id }, line = if (e.changed) "Device removed." else "That device was already removed.")
        is BridgeEvent.Failed -> when (e.cmd) {
            // These two have a screen waiting for an answer that will not come, so they end it.
            "join" -> s.copy(join = JoinState.Failed(e.message))
            "open" -> s.copy(ticket = null, ticketEnd = TicketEnd.Failed, line = e.message)
            else -> s.copy(line = e.message)
        }
        is BridgeEvent.Ping, is BridgeEvent.Keep -> s
        BridgeEvent.Bye -> s.copy(running = false, ticket = null, request = null)
    }

    /** The child ended: nothing it was showing is still true. */
    fun stopped(s: BridgeState): BridgeState = s.copy(
        running = false, ticket = null, request = null,
        // Nothing can be removed from a list the child is not there to change, and a join in flight is over.
        peers = null,
        join = if (s.join is JoinState.Connecting || s.join is JoinState.ShowDigits) JoinState.Failed(BRIDGE_STOPPED) else s.join,
    )

    const val BRIDGE_STOPPED = "The bridge stopped. Try again."

    /** A join left running when its screen closed is no longer something to show. */
    fun resetJoin(s: BridgeState): BridgeState = s.copy(join = JoinState.Idle)

    private const val MS_PER_SEC = 1_000L
}
