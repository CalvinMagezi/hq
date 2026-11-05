package example.hq.spike.measure

import example.hq.spike.testing.testWords
import example.hq.spike.ui.settings.SettingsWords
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class CopyDetailsTest {
    private fun column(version: String, since: String, vararg v: Pair<Counter, List<Long>>) = CounterColumn(version, since, v.toMap())

    private val facts = SettingsWords.DetailFacts("0.9.1", "1", "14", true, "Connected")

    private val now = column(
        "0.9.1", "2026-11-03",
        Counter.NotificationsAllowedFirstRun to listOf(1), Counter.AssistantChosen to listOf(1), Counter.LockedUnlocked to listOf(4),
        Counter.LockedCancelled to listOf(1), Counter.NoticePosted to listOf(12), Counter.NoticeTapped to listOf(7),
        Counter.NoticeServerTapped to listOf(3, 5), Counter.DataAppUpdates to listOf(0, 41_000_000), Counter.DataSpeechModel to listOf(0, 57_000_000),
        Counter.DataServerChecks to listOf(6_000_000, 2_000_000), Counter.DataOther to listOf(18_000_000, 210_000_000),
        Counter.UndoTaskDiscarded to listOf(2, 31), Counter.UndoNoteDeleted to listOf(0, 4), Counter.UndoHelperStopped to listOf(1, 3),
        Counter.SafeCopySaved to listOf(1), Counter.SavedCopyAge to listOf(12, 3, 1),
    )
    private val before = column(
        "0.9.0", "2026-10-20",
        Counter.NotificationsNotAllowed to listOf(1), Counter.NoticePosted to listOf(9), Counter.NoticeTapped to listOf(2),
        Counter.NoticeServerTapped to listOf(0, 2), Counter.DataServerChecks to listOf(9_000_000, 3_000_000),
        Counter.UndoTaskDiscarded to listOf(1, 20), Counter.StartWhileRunning to listOf(3), Counter.SavedCopyAge to listOf(8, 0, 0),
    )

    @Test fun theTextIsTheBoardsBlockOverAFixture() {
        val text = SettingsWords.copyDetails(testWords(), facts, CounterSnapshot(true, "", now, before))
        val expected = """
            AgentHQ 0.9.1 (build 1) on Android 14
            Background work: On
            HQ: Connected
            What HQ noticed (0.9.1 since 3 Nov | 0.9.0)
            notifications.allowed_first_run 1 | 0
            notifications.allowed_later 0 | 0
            notifications.not_allowed 0 | 1
            assistant.chosen yes | no
            locked_press.unlocked 4 | 0
            locked_press.cancelled 1 | 0
            answer_notice.posted 12 | 9
            answer_notice.tapped 7 | 2
            answer_notice.server_tapped 3/5 | 0/2
            data_mb.app_updates 0/41 | 0/0
            data_mb.speech_model 0/57 | 0/0
            data_mb.server_checks 6/2 | 9/3
            data_mb.other 18/210 | 0/0
            undo.task_discarded 2/31 | 1/20
            undo.note_deleted 0/4 | 0/0
            undo.helper_stopped 1/3 | 0/0
            start_hq.while_running 0 | 3
            start.closed_while_opening 0 | 0
            safe_copy.saved 1 | 0
            saved_copy.age 12/3/1 | 8/0/0

        """.trimIndent()
        assertEquals(expected, text)
    }

    @Test fun withoutALastVersionThereIsOneColumn() {
        val text = SettingsWords.copyDetails(testWords(), facts, CounterSnapshot(true, "", now, null))
        assertTrue(text.contains("What HQ noticed (0.9.1 since 3 Nov)\n"))
        assertTrue(text.contains("undo.task_discarded 2/31\n"))
        assertFalse(text.contains("|"))
    }

    @Test fun countingOffSaysSo() {
        val text = SettingsWords.copyDetails(testWords(), facts, CounterSnapshot(false, "2026-11-05", now, before))
        assertTrue(text.contains("What HQ noticed: counting is off, stopped on 5 Nov\n"))
    }

    @Test fun everyLineIsAKeyAndNumbersWhateverTheCountersHold() {
        val text = SettingsWords.copyDetails(testWords(), facts, CounterSnapshot(true, "", now, before)).lines().drop(4).filter { it.isNotBlank() }
        val keys = Counter.entries.map { it.key }
        assertEquals(keys.size, text.size)
        text.forEach { line -> assertTrue(line, Regex("""[a-z_.]+ [a-z0-9/]+( \| [a-z0-9/]+)?""").matches(line)) }
        assertTrue(text.map { it.substringBefore(' ') }.toSet() == keys.toSet())
    }

    @Test fun theBoardsSecondColumnReadsFromAnEmptyColumn() {
        val empty = column("0.9.0", "2026-10-20")
        val line = Noticed.lines.first { it.counter == Counter.DataSpeechModel }
        assertEquals("0/0", Noticed.token(empty, line))
        assertEquals("-", Noticed.token(null, line))
    }

    @Test fun theLineWordsHaveASentenceEach() {
        assertEquals(Counter.entries.toSet(), Noticed.lines.map { it.counter }.toSet())
        assertEquals(Noticed.lines.size, Noticed.lines.map { it.counter }.toSet().size)
    }

    @Test fun cellsReadAsTheBoardWritesThem() {
        val words = CellWords({ a, b -> "$a of $b" }, "Yes", "No")
        fun cell(c: Counter) = Noticed.cell(now, Noticed.lines.first { it.counter == c }, words)
        assertEquals("2 of 31", cell(Counter.UndoTaskDiscarded))
        assertEquals("6 \u00b7 2", cell(Counter.DataServerChecks))
        assertEquals("12 \u00b7 3 \u00b7 1", cell(Counter.SavedCopyAge))
        assertEquals("Yes", cell(Counter.AssistantChosen))
        assertEquals("4", cell(Counter.LockedUnlocked))
    }
}
