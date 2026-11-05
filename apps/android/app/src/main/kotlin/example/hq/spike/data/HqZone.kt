package example.hq.spike.data

import java.time.DateTimeException
import java.time.ZoneId
import org.json.JSONException
import org.json.JSONObject

/** The zone HQ keeps its daily times in. The phone shows every instant in its own zone and names this one only for a daily time. */
data class HqZone(val zone: ZoneId) {
    /** "Europe/London" reads "London", "America/Argentina/Buenos_Aires" reads "Buenos Aires". */
    val city: String get() = zone.id.substringAfterLast('/').replace('_', ' ')

    companion object {
        fun parse(body: String): HqZone? = try {
            val o = JSONObject(body)
            o.optString("tz").takeIf { it.isNotBlank() }?.let { HqZone(ZoneId.of(it)) }
        } catch (e: JSONException) {
            null
        } catch (e: DateTimeException) {
            null
        }
    }
}
