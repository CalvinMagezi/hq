package example.hq.spike.ui.tasks

import example.hq.spike.R
import example.hq.spike.ui.shell.Words
import java.time.Instant
import java.time.ZoneId
import org.json.JSONObject

/** One initiative's finished tasks. */
data class AwayFinished(val initiativeId: String, val name: String, val count: Int)

/** A task that failed a check and was reopened. */
data class AwayReopened(val taskId: String, val displayId: String, val title: String)

/** A task inside its finishing window, which ends at [releaseAtSec] (epoch seconds). */
data class AwayWindow(val taskId: String, val displayId: String, val title: String, val releaseAtSec: Long)

/** What HQ's checks did in the last 24 hours, as one HQ reports it. Read from the briefing; nothing here is worded. */
data class Away(
    val watching: Boolean,
    val finishedTotal: Int,
    val finished: List<AwayFinished>,
    val reopenedTotal: Int,
    val reopened: List<AwayReopened>,
    val windows: List<AwayWindow>,
    val waitingOnYou: Int,
) {
    /** The card shows only when there is something to say, and never while verification only watches. */
    val hasNews: Boolean get() = !watching && (finishedTotal > 0 || reopenedTotal > 0 || windows.isNotEmpty())

    companion object {
        fun parse(o: JSONObject): Away {
            val finished = o.optJSONArray("completed")?.let { a -> (0 until a.length()).map { a.getJSONObject(it) } }.orEmpty().map {
                AwayFinished(it.optString("initiative_id"), it.optString("initiative"), it.optInt("count"))
            }
            val reopened = o.optJSONArray("reworked")?.let { a -> (0 until a.length()).map { a.getJSONObject(it) } }.orEmpty().map {
                AwayReopened(it.optString("task_id"), it.optString("display_id"), it.optString("title"))
            }
            val windows = o.optJSONArray("veto_windows")?.let { a -> (0 until a.length()).map { a.getJSONObject(it) } }.orEmpty().map {
                AwayWindow(it.optString("task_id"), it.optString("display_id"), it.optString("title"), it.optLong("release_at"))
            }.sortedBy { it.releaseAtSec }
            return Away(
                watching = o.optString("mode", "off") != "enforce",
                finishedTotal = o.optInt("completed_total", finished.sumOf { it.count }),
                finished = finished,
                reopenedTotal = o.optInt("reworked_total", reopened.size),
                reopened = reopened,
                windows = windows,
                waitingOnYou = o.optInt("needs_you_total", o.optInt("needs_you")),
            )
        }
    }
}

/** The sentences of the card and the page, built from the briefing's counts. Zero parts are left out. */
object AwayWords {
    /** Who a reopened task went back to: the helper on it, else "you". Null means the owner. */
    fun sendBackTo(assignee: String?): String? = assignee?.trim()?.takeIf { it.isNotEmpty() && !it.equals("me", true) && !it.equals("you", true) }

    /** The card's lines, in the order finished, went back, finishing soon. [assignee] gives the helper on a task, by task id. */
    fun card(w: Words, a: Away, assignee: (String) -> String?, zone: ZoneId = ZoneId.systemDefault()): List<String> = buildList {
        if (a.finishedTotal > 0) add(w.n(R.plurals.away_finished, a.finishedTotal, a.finishedTotal))
        addAll(wentBack(w, a, assignee))
        if (a.windows.isNotEmpty()) add(w.n(R.plurals.away_finishes, a.windows.size, a.windows.size, w.clock(Instant.ofEpochSecond(a.windows.first().releaseAtSec), zone)))
    }

    /** "1 went back to you to finish.", then one line for each helper a task went back to. */
    fun wentBack(w: Words, a: Away, assignee: (String) -> String?): List<String> {
        if (a.reopenedTotal <= 0) return emptyList()
        val byHelper = a.reopened.groupingBy { sendBackTo(assignee(it.taskId)) }.eachCount()
        // Reopened tasks the briefing counted but did not list have no assignee to read, so they go to the owner.
        val unlisted = (a.reopenedTotal - a.reopened.size).coerceAtLeast(0)
        val toYou = (byHelper[null] ?: 0) + unlisted
        return buildList {
            if (toYou > 0) add(w.s(R.string.away_back_you, toYou))
            byHelper.filterKeys { it != null }.forEach { (helper, n) -> add(w.s(R.string.away_back_helper, n, helper!!)) }
        }
    }

    /** "Finishes at 18:00", "Finishes tomorrow at 09:00", or "Finishes Thu 7 Nov at 09:00" further out. */
    fun finishes(w: Words, releaseAtSec: Long, now: Instant, zone: ZoneId = ZoneId.systemDefault()): String {
        val at = Instant.ofEpochSecond(releaseAtSec)
        val days = java.time.temporal.ChronoUnit.DAYS.between(now.atZone(zone).toLocalDate(), at.atZone(zone).toLocalDate())
        return when {
            days <= 0 -> w.s(R.string.away_finishes_row, w.clock(at, zone))
            days == 1L -> w.s(R.string.away_finishes_tomorrow_row, w.clock(at, zone))
            else -> w.s(R.string.away_finishes_on_row, DAY_FORMAT.format(at.atZone(zone)), w.clock(at, zone))
        }
    }

    private val DAY_FORMAT = java.time.format.DateTimeFormatter.ofPattern("EEE d MMM", java.util.Locale.getDefault())
}
