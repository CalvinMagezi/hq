package example.hq.spike.ui.settings

import org.json.JSONObject

/** One kind of suggestion over the last [SuggestSummary.days] days, as HQ counted it. [says] is HQ's own wording for it. */
data class SuggestRow(val type: String, val says: String, val shown: Int, val tapped: Int, val notNow: Int, val later: Int, val never: Int)

data class SuggestSummary(val days: Int, val rows: List<SuggestRow>)

/** Reads `GET /api/offers/summary`. Anything that is not that answer is "HQ has not said yet" (null), never an error. */
object SuggestFacts {
    const val DEFAULT_DAYS = 30

    fun parse(body: String?): SuggestSummary? {
        val o = runCatching { JSONObject(body.orEmpty()) }.getOrNull() ?: return null
        val list = o.optJSONArray("types") ?: return null
        val rows = (0 until list.length()).mapNotNull { list.optJSONObject(it) }.mapNotNull { t ->
            val type = t.optString("type").trim()
            if (type.isEmpty()) return@mapNotNull null
            SuggestRow(
                type, t.optString("says").trim().ifEmpty { type }, t.optInt("shown"), t.optInt("tapped"), t.optInt("not_now"), t.optInt("later"), t.optInt("never"),
            )
        }
        return SuggestSummary(o.optInt("days", DEFAULT_DAYS).takeIf { it > 0 } ?: DEFAULT_DAYS, rows.filter { it.shown > 0 || it.never > 0 || it.notNow > 0 || it.later > 0 || it.tapped > 0 }.sortedByDescending { it.shown })
    }
}
