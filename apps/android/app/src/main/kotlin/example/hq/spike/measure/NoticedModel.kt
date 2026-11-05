package example.hq.spike.measure

import androidx.annotation.StringRes
import example.hq.spike.R
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import java.time.format.DateTimeParseException
import java.util.Locale

/** How a counter's slots read: one number, "x of y", mobile · Wi-Fi megabytes, three ages, or yes and no. */
enum class Shape { Number, OfTotal, Megabytes, Ages, Flag }

/** One line of "What HQ noticed": its words and where its numbers come from. */
data class NoticeLine(
    @StringRes val group: Int,
    @StringRes val label: Int,
    @StringRes val what: Int,
    @StringRes val only: Int,
    val counter: Counter,
    val shape: Shape,
    val asksPermission: Boolean = false,
)

/** The words a cell needs that depend on the language. */
class CellWords(val of: (Long, Long) -> String, val yes: String, val no: String)

/** The page's lines in order, and how each is written on the page and in Copy details. Pure, so it runs on the JVM. */
object Noticed {
    private const val ONE_MB = 1_000_000L
    private val ENGLISH_DAY = DateTimeFormatter.ofPattern("d MMM", Locale.ENGLISH)

    val lines: List<NoticeLine> = listOf(
        line(R.string.noticed_group_notifications, R.string.noticed_label_allowed_first_run, R.string.noticed_what_allowed_first_run, Counter.NotificationsAllowedFirstRun, Shape.Number, asks = true),
        line(R.string.noticed_group_notifications, R.string.noticed_label_allowed_later, R.string.noticed_what_allowed_later, Counter.NotificationsAllowedLater, Shape.Number, asks = true),
        line(R.string.noticed_group_notifications, R.string.noticed_label_not_allowed, R.string.noticed_what_not_allowed, Counter.NotificationsNotAllowed, Shape.Number, asks = true),
        line(R.string.noticed_group_assistant, R.string.noticed_label_assistant, R.string.noticed_what_assistant, Counter.AssistantChosen, Shape.Flag),
        line(R.string.noticed_group_assistant, R.string.noticed_label_unlocked, R.string.noticed_what_unlocked, Counter.LockedUnlocked, Shape.Number),
        line(R.string.noticed_group_assistant, R.string.noticed_label_cancelled, R.string.noticed_what_cancelled, Counter.LockedCancelled, Shape.Number),
        line(R.string.noticed_group_answers, R.string.noticed_label_posted, R.string.noticed_what_posted, Counter.NoticePosted, Shape.Number),
        line(R.string.noticed_group_answers, R.string.noticed_label_tapped, R.string.noticed_what_tapped, Counter.NoticeTapped, Shape.Number),
        line(R.string.noticed_group_answers, R.string.noticed_label_server_tapped, R.string.noticed_what_server_tapped, Counter.NoticeServerTapped, Shape.OfTotal),
        line(R.string.noticed_group_data, R.string.noticed_label_app_updates, R.string.noticed_what_data, Counter.DataAppUpdates, Shape.Megabytes),
        line(R.string.noticed_group_data, R.string.noticed_label_speech_model, R.string.noticed_what_data, Counter.DataSpeechModel, Shape.Megabytes),
        line(R.string.noticed_group_data, R.string.noticed_label_server_checks, R.string.noticed_what_data, Counter.DataServerChecks, Shape.Megabytes),
        line(R.string.noticed_group_data, R.string.noticed_label_other, R.string.noticed_what_data, Counter.DataOther, Shape.Megabytes),
        line(R.string.noticed_group_undo, R.string.noticed_label_task_discarded, R.string.noticed_what_undo_task_discarded, Counter.UndoTaskDiscarded, Shape.OfTotal, two = true),
        line(R.string.noticed_group_undo, R.string.noticed_label_note_deleted, R.string.noticed_what_undo_note_deleted, Counter.UndoNoteDeleted, Shape.OfTotal, two = true),
        line(R.string.noticed_group_undo, R.string.noticed_label_helper_stopped, R.string.noticed_what_undo_helper_stopped, Counter.UndoHelperStopped, Shape.OfTotal, two = true),
        line(R.string.noticed_group_starting, R.string.noticed_label_start_running, R.string.noticed_what_start_running, Counter.StartWhileRunning, Shape.Number),
        line(R.string.noticed_group_starting, R.string.noticed_label_closed_opening, R.string.noticed_what_closed_opening, Counter.StartClosedWhileOpening, Shape.Number),
        line(R.string.noticed_group_starting, R.string.noticed_label_safe_copy, R.string.noticed_what_safe_copy, Counter.SafeCopySaved, Shape.Number),
        line(R.string.noticed_group_saved_copy, R.string.noticed_label_saved_age, R.string.noticed_what_saved_age, Counter.SavedCopyAge, Shape.Ages),
    )

    private fun line(group: Int, label: Int, what: Int, counter: Counter, shape: Shape, two: Boolean = false, asks: Boolean = false) =
        NoticeLine(group, label, what, if (two) R.string.noticed_only_two else R.string.noticed_only_number, counter, shape, asks)

    /** A number for the page: "4", "2 of 31", "6 · 2" (mobile · Wi-Fi MB), "12 · 3 · 1" or Yes and No. A missing column reads "-". */
    fun cell(column: CounterColumn?, line: NoticeLine, words: CellWords): String {
        column ?: return "-"
        val c = line.counter
        return when (line.shape) {
            Shape.Number -> column.slot(c).toString()
            Shape.Flag -> if (column.slot(c) > 0) words.yes else words.no
            Shape.OfTotal -> words.of(column.slot(c, 0), column.slot(c, 1))
            Shape.Megabytes -> "${mb(column.slot(c, 0))} \u00b7 ${mb(column.slot(c, 1))}"
            Shape.Ages -> (0 until c.slots).joinToString(" \u00b7 ") { column.slot(c, it).toString() }
        }
    }

    /** The same numbers as one token of the Copy details block: "4", "2/31", "0/41", "12/3/1", "yes". */
    fun token(column: CounterColumn?, line: NoticeLine): String {
        column ?: return "-"
        val c = line.counter
        return when (line.shape) {
            Shape.Number -> column.slot(c).toString()
            Shape.Flag -> if (column.slot(c) > 0) "yes" else "no"
            Shape.Megabytes -> (0 until c.slots).joinToString("/") { mb(column.slot(c, it)).toString() }
            Shape.OfTotal, Shape.Ages -> (0 until c.slots).joinToString("/") { column.slot(c, it).toString() }
        }
    }

    /** Megabytes, rounded; "give or take 1" is the board's own tolerance. */
    fun mb(bytes: Long): Long = (bytes + ONE_MB / 2) / ONE_MB

    /** "3 Nov" from an ISO date, or the text as it is when it is not one. */
    fun day(iso: String, locale: Locale = Locale.getDefault()): String = try {
        LocalDate.parse(iso).format(DateTimeFormatter.ofPattern("d MMM", locale))
    } catch (_: DateTimeParseException) {
        iso
    }

    /** The block of Copy details: English keys and one number token per column, so anyone can read it. Only counts, never content. */
    fun block(snap: CounterSnapshot): String = buildString {
        if (!snap.on) {
            appendLine("What HQ noticed: counting is off, stopped on ${dayEnglish(snap.stoppedOn)}")
        } else {
            val last = snap.previous?.let { " | ${it.version}" }.orEmpty()
            appendLine("What HQ noticed (${snap.current.version} since ${dayEnglish(snap.current.since)}$last)")
        }
        for (l in lines) appendLine("${l.counter.key} ${token(snap.current, l)}" + (snap.previous?.let { " | ${token(it, l)}" } ?: ""))
    }

    private fun dayEnglish(iso: String) = try {
        LocalDate.parse(iso).format(ENGLISH_DAY)
    } catch (_: DateTimeParseException) {
        iso
    }
}
