package example.hq.spike.ui.pairing

import example.hq.spike.bridge.BridgePeer
import example.hq.spike.bridge.BridgeState
import example.hq.spike.bridge.TicketEnd

/** The words of the pairing screens. Pure, so each sentence is tested from the state that makes it. */
object PairingWords {
    const val TITLE = "Pair a device"
    const val INTRO = "A device you pair can reach this phone directly, with no account and no VPN. Only devices you pair here can."
    const val SHOW_TITLE = "Show a code"
    const val SHOW_HELP = "On the other device, scan this code, or paste the link."
    const val SCAN_TITLE = "Scan a code"
    const val SCAN_HELP = "Point the camera at the code on the other device."
    const val NOT_OURS = "That is not an HQ pairing code."
    const val CAMERA_NEEDED = "Scanning needs the camera. You can paste the link instead."
    const val CONFIRM_TITLE = "Pair with another device?"
    const val CONFIRM_BODY = "A link asks this phone to pair with another device. Go on only if you just showed this code on a device of yours. Anyone can send you a link like this."
    const val PASTE_TITLE = "Paste a link"
    const val PASTE_HINT = "hqpair://ticket/…"
    const val DIGITS_HELP = "Type these six digits on the other device. It pairs only if they match."
    const val TYPE_HELP = "Type the six digits it shows."
    const val BATTERY_NOTICE = "Android may stop this phone from answering after it has been asleep for a long time. Turn off battery optimisation for AgentHQ to prevent that."
    const val BATTERY_BUTTON = "Open battery settings"
    const val DIGITS_LENGTH = 6
    private const val SECONDS_PER_MINUTE = 60L
    private const val MS_PER_SEC = 1_000L
    private const val MINUTES_PER_HOUR = 60L
    private const val HOURS_PER_DAY = 24L

    fun switchLine(s: BridgeState): String = when {
        !s.enabled -> "Off. Nothing can reach this phone."
        !s.running -> "Starting…"
        else -> "On. Paired devices can reach this phone as ${PairLink.shortId(s.deviceId ?: "")}."
    }

    /** The battery notice is for a phone that is meant to be reached and that Android is allowed to pause. */
    fun showBatteryNotice(s: BridgeState, ignoringOptimisation: Boolean): Boolean = s.enabled && !ignoringOptimisation

    fun relayLine(relay: String?): String? = when (relay) {
        null, "" -> null
        "none" -> "No relay: devices reach each other directly or not at all."
        "default" -> "Using n0's public relays, which are for development and testing."
        else -> "Using your relay."
    }

    fun countdown(msLeft: Long): String {
        val total = (msLeft.coerceAtLeast(0) + MS_PER_SEC - 1) / MS_PER_SEC
        return "%d:%02d".format(total / SECONDS_PER_MINUTE, total % SECONDS_PER_MINUTE)
    }

    fun wants(name: String, joiner: String): String = "“${name.ifBlank { "A device" }}” (${PairLink.shortId(joiner)}) wants to pair. $TYPE_HELP"

    fun ended(e: TicketEnd): String = when (e) {
        TicketEnd.Paired -> "Paired."
        TicketEnd.Declined -> "The digits did not match, so nothing was paired. Show a new code to try again."
        TicketEnd.TooLate -> "The other device gave up waiting. Show a new code to try again."
        TicketEnd.Expired -> "The code ran out. Show a new one."
    }

    fun joined(name: String, id: String): String = "Paired with “${name.ifBlank { "a device" }}” (${PairLink.shortId(id)}). Check that this is the device you meant."

    fun peerLine(p: BridgePeer, nowSecs: Long): String {
        val seen = p.lastSeenAt?.let { "seen ${ago(nowSecs - it)}" } ?: "not seen yet"
        return "${PairLink.shortId(p.id)} · $seen"
    }

    fun ago(secs: Long): String {
        val s = secs.coerceAtLeast(0)
        val minutes = s / SECONDS_PER_MINUTE
        val hours = minutes / MINUTES_PER_HOUR
        return when {
            s < SECONDS_PER_MINUTE -> "just now"
            minutes < MINUTES_PER_HOUR -> "$minutes min ago"
            hours < HOURS_PER_DAY -> "$hours h ago"
            else -> "${hours / HOURS_PER_DAY} d ago"
        }
    }

    fun removeTitle(p: BridgePeer) = "Remove “${p.name.ifBlank { PairLink.shortId(p.id) }}”?"
    const val REMOVE_BODY = "It loses access to this phone at once and its open connection is closed. You can pair it again later."

    /** Digits only, at most six: what a field may hold while the person types. */
    fun typed(text: String): String = text.filter(Char::isDigit).take(DIGITS_LENGTH)
}
