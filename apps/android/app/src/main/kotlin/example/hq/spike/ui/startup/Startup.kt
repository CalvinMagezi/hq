package example.hq.spike.ui.startup

import android.content.Context
import example.hq.spike.data.HqStatus
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** Whether the app has ever heard HQ answer: the first start has nothing cached to show, so it gets its own screen. */
object FirstLaunch {
    private const val FILE = "startup"
    private const val DONE = "first_done"

    fun isDone(c: Context): Boolean = c.getSharedPreferences(FILE, Context.MODE_PRIVATE).getBoolean(DONE, false)
    fun markDone(c: Context) { val p = c.getSharedPreferences(FILE, Context.MODE_PRIVATE); if (!p.getBoolean(DONE, false)) p.edit().putBoolean(DONE, true).apply() }
}

/** Which screen covers the app while HQ is not answering. */
enum class Startup { None, Starting, DidNotStart }

object StartupWords {
    /** After this many seconds without a healthy answer, the start screen gives way to "HQ did not start". */
    const val GIVE_UP_SECONDS = 60L

    fun decide(status: HqStatus, firstLaunchDone: Boolean): Startup = when {
        status !is HqStatus.Starting -> Startup.None
        status.seconds >= GIVE_UP_SECONDS -> Startup.DidNotStart
        !firstLaunchDone -> Startup.Starting
        else -> Startup.None
    }

    const val STARTING_TITLE = "Starting HQ"
    const val STARTING_BODY = "HQ is starting on this phone. The first start after a reboot can take up to a minute."
    const val CAPTURE_HINT = "Have a thought? It is saved now and sent when HQ is up."
    const val FAILED_TITLE = "HQ did not start"
    const val FAILED_BODY = "Android may have stopped it to save battery, or the phone ran low on memory. Nothing you captured is lost."
    const val STARTING_BANNER = "HQ is starting. Your draft is kept; Send turns on when it is up."

    /** The two lines in the box: what timed out, and what is known about the last start. */
    fun failedLines(lastStartMs: Long, exitCode: Int?, zone: ZoneId = ZoneId.systemDefault()): List<String> {
        val start = if (lastStartMs <= 0) "never" else java.time.Instant.ofEpochMilli(lastStartMs).atZone(zone).toLocalTime().format(DateTimeFormatter.ofPattern("HH:mm"))
        return listOf("Health check timed out after $GIVE_UP_SECONDS s.", "Last start: $start \u00b7 exit code ${exitCode?.toString() ?: "none"}")
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
