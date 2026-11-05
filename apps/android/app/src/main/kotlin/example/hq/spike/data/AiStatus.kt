package example.hq.spike.data

import org.json.JSONObject

/** What `GET /api/status/ai` says: whether HQ's AI services answer, and the newest one that does not. Never holds prompt or answer text. */
data class AiStatus(val answering: Boolean, val failingService: String? = null, val failedAtMs: Long? = null, val kind: String? = null) {
    companion object {
        /** HQ sends times in epoch seconds. A reply this app cannot read counts as "answering": nothing says it is down. */
        fun parse(text: String): AiStatus = try {
            val o = JSONObject(text)
            val failing = o.optJSONArray("services").objects()
                .filter { !it.isNull("last_failed_at") && it.optLong("last_failed_at") > it.optLong("last_ok_at", 0L) }
                .maxByOrNull { it.optLong("last_failed_at") }
            if (o.optBoolean("answering", true) || failing == null) AiStatus(true)
            else AiStatus(false, failing.optString("provider").ifBlank { null }, failing.optLong("last_failed_at") * MS, failing.optString("failure").ifBlank { null })
        } catch (e: org.json.JSONException) {
            AiStatus(true)
        }

        private const val MS = 1000L
    }
}
