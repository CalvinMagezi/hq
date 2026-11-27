package example.hq.spike.data

import org.json.JSONArray
import org.json.JSONObject

// The reminder calls of [HqClient] (`/api/reminders`, docs/REMINDERS.md).

/** One reminder as HQ sends it. [dueEpoch] is when it next rings, in epoch seconds; [due] is the same in the owner's words. */
data class Reminder(
    val id: String,
    val title: String,
    val note: String?,
    val status: String,
    val due: String,
    val dueEpoch: Long,
    val timeZone: String,
    /** "every day", "every Mon, Thu" and so on; null for one that rings once. */
    val repeat: String?,
    /** Keeps reminding until it is marked done. */
    val nudge: Boolean,
    val pings: Int,
    val tainted: Boolean,
)

/** The list, with the zone the owner saved (null until a phone reports one) and HQ's own clock in epoch seconds. */
data class ReminderList(val timeZone: String?, val now: Long, val reminders: List<Reminder>)

internal fun parseReminder(o: JSONObject) = Reminder(
    id = o.getString("id"),
    title = o.optString("title", ""),
    note = o.optStringOrNull("note"),
    status = o.optString("status", "active"),
    due = o.optString("due", ""),
    dueEpoch = o.optLong("due_epoch", 0L),
    timeZone = o.optString("time_zone", ""),
    repeat = o.optStringOrNull("repeat"),
    nudge = o.optBoolean("nudge", false),
    pings = o.optInt("pings", 0),
    tainted = o.optBoolean("tainted", false),
)

internal fun parseReminderList(text: String): ReminderList {
    val o = JSONObject(text)
    val rows: JSONArray = o.optJSONArray("reminders") ?: JSONArray()
    return ReminderList(o.optStringOrNull("time_zone"), o.optLong("now", 0L), (0 until rows.length()).mapNotNull { rows.optJSONObject(it)?.let(::parseReminder) })
}

/** [done] false is what is coming (active), true what finished or was stopped. */
suspend fun HqClient.reminders(done: Boolean = false): ReminderList =
    send(authed("/api/reminders?status=${if (done) "done" else "active"}").build()) { parseReminderList(it) }

/** Sets a reminder. [at] is local time in [zone], `YYYY-MM-DDTHH:MM`; [repeat] is HQ's short form (`daily`, `weekly:5`). */
suspend fun HqClient.createReminder(title: String, at: String, zone: String, repeat: String?, nudge: Boolean): Reminder {
    val body = JSONObject().put("title", title).put("at", at).put("tz", zone).put("nudge", nudge)
    if (repeat != null) body.put("repeat", repeat)
    return send(authed("/api/reminders").post(json(body)).build()) { parseReminder(JSONObject(it)) }
}

suspend fun HqClient.doneReminder(id: String) {
    send(authed("/api/reminders/${java.net.URLEncoder.encode(id, "UTF-8")}/done").post(HqClient.EMPTY_JSON).build()) { }
}

/** Rings again at [until] (epoch seconds). */
suspend fun HqClient.snoozeReminder(id: String, until: Long) {
    send(authed("/api/reminders/${java.net.URLEncoder.encode(id, "UTF-8")}/snooze").post(json(JSONObject().put("until", until))).build()) { }
}

suspend fun HqClient.stopReminder(id: String) {
    send(authed("/api/reminders/${java.net.URLEncoder.encode(id, "UTF-8")}").delete().build()) { }
}
