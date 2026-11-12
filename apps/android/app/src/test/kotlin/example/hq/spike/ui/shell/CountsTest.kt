package example.hq.spike.ui.shell

import example.hq.spike.NoticeWords
import example.hq.spike.ui.inbox.InboxCard
import example.hq.spike.ui.tasks.TasksLogic
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import example.hq.spike.testing.testWords

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class CountsTest {
    private val nb = '\u00a0'

    @Test fun progressKeepsTheCountTogether() {
        assertEquals("1${nb}of${nb}3${nb}sub-tasks${nb}done", Counts.progress(testWords(), 1, 3))
        assertEquals("1${nb}of${nb}3${nb}done", Counts.progress(testWords(), 1, 3, subTasks = false))
    }

    @Test fun badgesStopAtNinetyNine() {
        assertEquals("99", Counts.badge(99))
        assertEquals("99+", Counts.badge(100))
        assertEquals("Show all${nb}12", Counts.showAll(testWords(), 12))
        assertEquals("1${nb}task", Counts.tasks(testWords(), 1))
        assertEquals("2${nb}tasks", Counts.tasks(testWords(), 2))
    }

    @Test fun theBarBadgeStopsAtNine() {
        assertEquals("1", Counts.navBadge(1))
        assertEquals("9", Counts.navBadge(9))
        assertEquals("9+", Counts.navBadge(10))
        assertEquals("9+", Counts.navBadge(162))
    }

    @Test fun noSubtasksSaysNoneYet() {
        assertEquals("None yet", TasksLogic.subtaskLabel(testWords(), 0, 0))
        assertEquals("1${nb}of${nb}3${nb}done", TasksLogic.subtaskLabel(testWords(), 1, 3))
    }

    @Test fun theOldWordsAreGoneFromTheSource() {
        val root = File("src/main/kotlin/example/hq/spike/ui")
        val text = root.walkTopDown().filter { it.extension == "kt" && ("tasks" in it.path || "search" in it.path || "shell" in it.path) }.joinToString("\n") { it.readText() }
        assertFalse(Regex("of \\$\\{[^}]*\\} steps").containsMatchIn(text))
        assertFalse("hq is working" in text)
        assertFalse("hq finished" in text)
        assertFalse("See all" in text)
        assertTrue(text.isNotBlank())
    }

    @Test fun losingComesFromTheFlagNotTheWords() {
        val card = InboxCard("1", example.hq.spike.ui.inbox.CardKind.Approval, "Delete everything", "This will delete everything", "", "2025-01-01T00:00:00Z", example.hq.spike.ui.inbox.CardIcon.Info, null, true, "Allow", null, "", "pending")
        assertFalse(NoticeWords.losing(card))
        assertTrue(NoticeWords.losing(card.copy(loses = true, verb = "delete")))
    }
}
