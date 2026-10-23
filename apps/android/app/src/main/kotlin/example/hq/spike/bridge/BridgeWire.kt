package example.hq.spike.bridge

import org.json.JSONArray
import org.json.JSONObject

/** One paired device, as `hq bridge run` reports it. [name] is text the other device chose: show it, never trust it. */
/** Times are seconds since 1970, as the database keeps them. */
data class BridgePeer(val id: String, val name: String, val pairedAt: Long, val lastSeenAt: Long?, val revokedAt: Long?) {
    val active: Boolean get() = revokedAt == null
}

/** What `hq bridge run` says on its standard output, one JSON object per line. */
sealed interface BridgeEvent {
    data class Ready(val id: String, val name: String, val relay: String) : BridgeEvent
    /** The pairing link of the open ticket. It holds the one-time code: show it, never log it. */
    data class Ticket(val link: String, val expiresInSecs: Int) : BridgeEvent
    /** Someone scanned the ticket and waits for the six digits their screen shows. */
    data class Request(val joiner: String, val name: String, val windowSecs: Int) : BridgeEvent
    data class Decision(val joiner: String, val result: String) : BridgeEvent
    data class TicketClosed(val paired: Boolean) : BridgeEvent
    /** The six digits to show while this phone joins; the person types them on the other device. */
    data class Digits(val value: String) : BridgeEvent
    data class Joined(val id: String, val name: String) : BridgeEvent
    data class JoinFailed(val why: String) : BridgeEvent
    data class Peers(val peers: List<BridgePeer>) : BridgeEvent
    data class Revoked(val id: String, val changed: Boolean) : BridgeEvent
    data class Ping(val id: String, val connectMs: Long?, val medianMs: Long?, val path: String?, val error: String?) : BridgeEvent
    data class Keep(val id: String, val up: Boolean, val rttMs: Long?, val path: String?, val downMs: Long?) : BridgeEvent
    data class Failed(val cmd: String, val message: String) : BridgeEvent
    data object Bye : BridgeEvent
}

/** The two lines of the control protocol (`docs/HQ_MOBILE.md`, "The bridge"). Pure, so each line is tested from a real example. */
object BridgeWire {
    const val DEFAULT_TICKET_SECS = 600

    /** Null for a line that is not a known event, so a newer `hq` can add events without breaking an older app. */
    fun parse(line: String): BridgeEvent? {
        val o = try { JSONObject(line) } catch (e: org.json.JSONException) { return null }
        return when (o.optString("event")) {
            "ready" -> BridgeEvent.Ready(o.optString("id"), o.optString("name"), o.optString("relay"))
            "ticket" -> BridgeEvent.Ticket(o.optString("link"), o.optInt("expires_in_secs"))
            "request" -> BridgeEvent.Request(o.optString("joiner"), o.optString("name"), o.optInt("window_secs"))
            "decision" -> BridgeEvent.Decision(o.optString("joiner"), o.optString("result"))
            "ticket_closed" -> BridgeEvent.TicketClosed(o.optBoolean("paired"))
            "digits" -> BridgeEvent.Digits(o.optString("value"))
            "joined" -> BridgeEvent.Joined(o.optString("id"), o.optString("name"))
            "join_failed" -> BridgeEvent.JoinFailed(o.optString("why"))
            "peers" -> BridgeEvent.Peers(peers(o.optJSONArray("peers")))
            "revoked" -> BridgeEvent.Revoked(o.optString("id"), o.optBoolean("changed"))
            "ping" -> BridgeEvent.Ping(o.optString("id"), o.longOrNull("connect_ms"), o.longOrNull("median_ms"), o.stringOrNull("path"), o.stringOrNull("error"))
            "keep" -> BridgeEvent.Keep(o.optString("id"), o.optString("state") == "up", o.longOrNull("rtt_ms"), o.stringOrNull("path"), o.longOrNull("down_ms"))
            "error" -> BridgeEvent.Failed(o.optString("cmd"), o.optString("message"))
            "bye" -> BridgeEvent.Bye
            else -> null
        }
    }

    private fun peers(a: JSONArray?): List<BridgePeer> = (0 until (a?.length() ?: 0)).mapNotNull { i ->
        val p = a?.optJSONObject(i) ?: return@mapNotNull null
        BridgePeer(p.optString("id"), p.optString("name"), p.optLong("paired_at"), p.longOrNull("last_seen_at"), p.longOrNull("revoked_at"))
    }

    private fun JSONObject.stringOrNull(name: String): String? = if (has(name) && !isNull(name)) optString(name) else null
    private fun JSONObject.longOrNull(name: String): Long? = if (has(name) && !isNull(name)) optLong(name) else null

    fun open(ttlSecs: Int = DEFAULT_TICKET_SECS) = line("open") { put("ttl_secs", ttlSecs) }
    fun cancel() = line("cancel")
    fun digits(value: String) = line("digits") { put("value", value) }
    fun join(link: String) = line("join") { put("link", link) }
    fun peers() = line("peers")
    fun revoke(id: String) = line("revoke") { put("id", id) }
    fun networkChange() = line("network_change")
    fun ping(id: String, count: Int = 3) = line("ping") { put("id", id); put("count", count) }
    fun keep(id: String) = line("keep") { put("id", id) }
    fun keepStop() = line("keep_stop")
    fun quit() = line("quit")

    private fun line(cmd: String, extra: JSONObject.() -> Unit = {}): String = JSONObject().put("cmd", cmd).apply(extra).toString()
}
