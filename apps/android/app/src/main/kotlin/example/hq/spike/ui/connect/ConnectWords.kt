package example.hq.spike.ui.connect

/** Where the address on the screen came from. */
enum class UrlSource { Hub, Stub }

/**
 * What the screen knows about the connector. [url] is what hq says Claude should be given when the
 * connector is on; until the edge and the device's own TLS exist (workstreams W1 and W2) it can come from
 * a stand-in file instead, and then [source] says so and arming stays off.
 */
data class ConnectFacts(
    val enabled: Boolean = false,
    val url: String? = null,
    val source: UrlSource? = null,
    /** Seconds since 1970 when the arming window ends, or null. */
    val armedUntil: Long? = null,
    val clients: Int = 0,
)

/** The words of the Connect Claude screen. Pure, so each sentence is tested from the facts that make it. */
object ConnectWords {
    const val TITLE = "Connect Claude"
    const val ARM_MINUTES = 10
    private const val SECONDS_PER_MINUTE = 60L

    fun armed(f: ConnectFacts, nowSecs: Long): Boolean = f.armedUntil?.let { it > nowSecs } == true

    /** Whether Arm may be pressed: the connector has to be on in hq itself, not just shown from a stand-in. */
    fun canArm(f: ConnectFacts): Boolean = f.enabled && f.source == UrlSource.Hub

    fun intro(f: ConnectFacts): String = when {
        f.url == null -> "Claude reaches this phone through an edge: a small public forwarder that passes encrypted traffic and cannot read it. The address to give Claude shows here once the connector is on."
        f.source == UrlSource.Stub -> "This is a stand-in address. The connector is not on in HQ yet, so Claude cannot use it."
        else -> "Add this address as a connector in Claude, then arm it here and press Connect in Claude."
    }

    fun status(f: ConnectFacts, nowSecs: Long): String = when {
        !f.enabled -> "Off. Nothing can connect."
        armed(f, nowSecs) -> "Armed for ${countdown((f.armedUntil ?: nowSecs) - nowSecs)} more. Press Connect in Claude now."
        f.clients == 0 -> "On, and nothing is connected. Arm it to let Claude in."
        f.clients == 1 -> "On. One client is connected."
        else -> "On. ${f.clients} clients are connected."
    }

    fun countdown(secsLeft: Long): String {
        val s = secsLeft.coerceAtLeast(0)
        return "%d:%02d".format(s / SECONDS_PER_MINUTE, s % SECONDS_PER_MINUTE)
    }

    /** The stand-in file holds one https address; anything else is ignored rather than shown as an address to give Claude. */
    fun standIn(text: String?): String? = address(text?.lineSequence()?.firstOrNull())

    /** An https address without spaces, of a sensible length, or null. */
    fun address(text: String?): String? {
        val t = text?.trim() ?: return null
        return t.takeIf { it.length in MIN_URL..MAX_URL && it.startsWith("https://") && it.none(Char::isWhitespace) }
    }

    private const val MIN_URL = 12
    private const val MAX_URL = 200
}
