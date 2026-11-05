package example.hq.spike.ui.startup

import android.content.Context
import example.hq.spike.R
import example.hq.spike.data.Connection
import java.time.ZoneId
import example.hq.spike.ui.shell.Words

/** Whether the app has ever heard HQ answer: the first start has nothing cached to show, so it gets its own screen. */
object FirstLaunch {
    private const val FILE = "startup"
    private const val DONE = "first_done"

    fun isDone(c: Context): Boolean = c.getSharedPreferences(FILE, Context.MODE_PRIVATE).getBoolean(DONE, false)
    fun markDone(c: Context) { val p = c.getSharedPreferences(FILE, Context.MODE_PRIVATE); if (!p.getBoolean(DONE, false)) p.edit().putBoolean(DONE, true).apply() }
}

/** Which screen covers the app while HQ is not answering. */
enum class Startup { None, Starting }

/** The facts of one start that the cover, the sheet and the notice read. */
object StartRules {
    const val STEPS = 4
    const val SLOW_FLOOR_S = 30L
    const val STILL_OPENING_S = 60L
    const val LOW_SPACE_BYTES = 200_000_000L
    const val USUAL_ROUND_S = 5L
    const val USUAL_KEPT = 5
    private const val DAY_MS = 24 * 60 * 60 * 1000L

    /** The cover only on the first launch (or when the signed switch brings the old cover back); later starts keep the app usable. */
    fun decide(status: Connection, firstLaunchDone: Boolean, coverRemoval: Boolean = true): Startup = when {
        status !is Connection.Starting -> Startup.None
        !firstLaunchDone || !coverRemoval -> Startup.Starting
        else -> Startup.None
    }

    /** The step (1 to 4) from what the phone can see: the hub is up, hq's process lives, its port answers a connection. */
    fun step(hubUp: Boolean, alive: Boolean, portOpen: Boolean): Int = when {
        !hubUp -> 1
        !alive -> 2
        !portOpen -> 3
        else -> 4
    }

    /** The median of the last five good starts in seconds, rounded to 5 s, shown from the second start on. */
    fun usual(startsMs: List<Long>): Long? {
        val last = startsMs.takeLast(USUAL_KEPT)
        if (last.size < 2) return null
        val sorted = last.sorted()
        val median = if (sorted.size % 2 == 1) sorted[sorted.size / 2] else (sorted[sorted.size / 2 - 1] + sorted[sorted.size / 2]) / 2
        return maxOf(USUAL_ROUND_S, Math.round(median / 1000.0 / USUAL_ROUND_S) * USUAL_ROUND_S)
    }

    /** Past twice the usual time and never before 30 s. */
    fun slow(seconds: Long, usualSeconds: Long?): Boolean = usualSeconds != null && seconds >= SLOW_FLOOR_S && seconds > 2 * usualSeconds

    fun stillOpening(seconds: Long, step: Int): Boolean = step == 3 && seconds >= STILL_OPENING_S

    fun almostFull(freeBytes: Long?): Boolean = freeBytes != null && freeBytes < LOW_SPACE_BYTES

    /** The sheet opens by itself at most once a day when HQ has stopped. */
    fun autoOpenOk(lastMs: Long, nowMs: Long): Boolean = lastMs <= 0 || nowMs - lastMs >= DAY_MS

    private const val FILE = "startup"
    private const val KEY_STARTS = "starts_ms"
    private const val KEY_ANSWER = "last_answer"
    private const val KEY_AUTO = "stopped_sheet_at"

    fun starts(c: Context): List<Long> = c.getSharedPreferences(FILE, Context.MODE_PRIVATE).getString(KEY_STARTS, "").orEmpty().split(',').mapNotNull { it.toLongOrNull() }

    fun recordStart(c: Context, ms: Long) {
        val kept = (starts(c) + ms).takeLast(USUAL_KEPT)
        c.getSharedPreferences(FILE, Context.MODE_PRIVATE).edit().putString(KEY_STARTS, kept.joinToString(",")).apply()
    }

    /** When HQ last answered: the time of the copy the app holds while it is down. */
    fun lastAnswer(c: Context): Long = c.getSharedPreferences(FILE, Context.MODE_PRIVATE).getLong(KEY_ANSWER, 0)
    fun markAnswer(c: Context, ms: Long) { c.getSharedPreferences(FILE, Context.MODE_PRIVATE).edit().putLong(KEY_ANSWER, ms).apply() }

    fun lastAutoOpen(c: Context): Long = c.getSharedPreferences(FILE, Context.MODE_PRIVATE).getLong(KEY_AUTO, 0)
    fun markAutoOpen(c: Context, ms: Long) { c.getSharedPreferences(FILE, Context.MODE_PRIVATE).edit().putLong(KEY_AUTO, ms).apply() }
}

object StartupWords {
    /** Technical details: what timed out and what is known about the last start. */
    fun failedLines(w: Words, lastStartMs: Long, exitCode: Int?, timeoutS: Long, zone: ZoneId = ZoneId.systemDefault()): List<String> {
        val start = if (lastStartMs <= 0) w.s(R.string.conn_exit_none) else w.clock(java.time.Instant.ofEpochMilli(lastStartMs), zone)
        return listOf(w.s(R.string.conn_timeout, timeoutS), w.s(R.string.conn_last_start, start) + " \u00b7 " + w.s(R.string.conn_exit_code, exitCode?.toString() ?: w.s(R.string.conn_exit_none)))
    }

    private val SECRET = Regex("[A-Za-z0-9_\\-]{32,}")

    /** Log lines shortened and with anything that looks like a token blanked, so a copied report is safe to share. */
    fun cleanLog(lines: List<String>, keep: Int = 12): List<String> = lines.takeLast(keep).map { SECRET.replace(it.take(200), "\u2026") }

    /** What "Copy details" puts on the clipboard. */
    fun details(build: String, active: Boolean, lines: List<String>, log: List<String>): String = buildString {
        appendLine("AgentHQ $build")
        appendLine("Run mode: ${if (active) "Active" else "Standby"}")
        lines.forEach { appendLine(it) }
        if (log.isNotEmpty()) { appendLine(); appendLine("Recent hq log:"); log.forEach { appendLine(it) } }
    }
}
