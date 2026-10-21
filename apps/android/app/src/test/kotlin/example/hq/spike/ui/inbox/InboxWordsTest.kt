package example.hq.spike.ui.inbox

import example.hq.spike.data.HqClient.InboxItem
import java.time.Instant
import java.time.ZoneOffset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class InboxWordsTest {
    private fun item(title: String, description: String, actions: List<String> = listOf("approve", "reject"), source: String? = null, kind: String = "action_needed") =
        InboxItem("val_1", kind, title, description, "pending", "2026-10-21T12:00:00+00:00", null, source, null, actions)

    private val blocked = """
        Session s-42 on mac-mini is blocked on a prompt or approval:
        <hq-output>
        Do you want to edit Chat.dc.html?
        1. Yes
        2. No
        </hq-output>
        Read it with `harness_session_logs`, answer with `harness_session_send`.
    """.trimIndent()

    @Test fun aHelperAtAPromptBecomesOneOkAndNamesTheSession() {
        val c = InboxWords.card(item("Harness session 'claude in hq-oss' is waiting for you", blocked, source = "session-supervisor"))
        assertEquals(CardKind.HelperAsks, c.kind)
        assertEquals("Claude needs your OK", c.title)
        assertEquals("s-42", c.sessionId)
        assertEquals("mac-mini", c.where)
        assertTrue(c.canAllow)
        assertEquals("Allow once", c.allowLabel)
        assertEquals("Wants to edit one file: Chat.dc.html", c.body)
        assertEquals(listOf("Do you want to edit Chat.dc.html?", "1. Yes", "2. No"), c.excerpt)
    }

    private fun bashNotice(vararg command: String) = "Session s-7 on mac-mini is blocked on a prompt or approval:\n<hq-output>\n" +
        (listOf("Bash command") + command + listOf("Do you want to proceed?", "❯ 1. Yes", "  2. No")).joinToString("\n") + "\n</hq-output>\nRead it with x"

    @Test fun aShortCommandIsShownOnTheCardAndCanBeAnsweredThere() {
        val c = InboxWords.card(item("Harness session 'claude in x' is waiting for you", bashNotice("ls -la"), source = "session-supervisor"))
        assertTrue(c.canAllow)
        assertEquals("Allow once", c.allowLabel)
        assertTrue(c.body.contains("ls -la"))
    }

    @Test fun aLongCommandOrAPlanIsNotAnsweredFromTheCardOrTheShade() {
        val long = InboxWords.card(item("Harness session 'claude in x' is waiting for you", bashNotice("rm -rf build", "--flag-one --flag-two", "--flag-three --flag-four", "> /tmp/out.log"), source = "session-supervisor"))
        assertFalse(long.canAllow)
        assertEquals("Open helper", long.allowLabel)
        assertEquals("s-7", long.sessionId)
        val plan = "Session s-8 on mac is blocked on a prompt or approval:\n<hq-output>\nWould you like to proceed?\n❯ 1. Yes, and auto-accept edits\n  2. No\n</hq-output>"
        val p = InboxWords.card(item("Harness session 'claude in x' is waiting for you", plan, source = "session-supervisor"))
        assertFalse(p.canAllow)
        assertEquals("Open helper", p.allowLabel)
    }

    @Test fun aNoticeThatQuotesNoQuestionCannotBeAnsweredFromTheCard() {
        val c = InboxWords.card(item("Harness session 'claude in x' is waiting for you", "Session s-9 on mac is blocked on a prompt or approval:\nRead it with x", source = "session-supervisor"))
        assertFalse(c.canAllow)
        assertEquals("Open helper", c.allowLabel)
        assertEquals(emptyList<String>(), c.excerpt)
    }

    @Test fun aHelperWithNoSessionInTheTextCannotBeAllowedFromHere() {
        val c = InboxWords.card(item("Harness session 'codex' is waiting for you", "Something went wrong"))
        assertFalse(c.canAllow)
    }

    @Test fun aChatHelperThatNeedsAnAnswerIsLookedAt() {
        val c = InboxWords.card(item("Harness session 'codex' needs an answer", "Session s-7 on mac-mini is waiting at a prompt. Its chat has the details.", source = "session_chat"))
        assertEquals(CardKind.HelperStuck, c.kind)
        assertFalse(c.canAllow)
        assertEquals("Take a look", c.allowLabel)
    }

    @Test fun anApprovalKeepsItsOwnWordsAndAllowsOnlyWhenShort() {
        assertTrue(InboxWords.card(item("Delete 3 old backups?", "The nightly backup wants to remove the 3 oldest copies.", source = "backup-task")).canAllow)
        val long = InboxWords.card(item("Run this?", "x".repeat(500)))
        assertFalse(long.canAllow)
        assertTrue(long.body.length <= 401)
    }

    @Test fun anUpdateHasNoAnswerAndAFittingIcon() {
        val c = InboxWords.card(item("Backup finished", "2,860 notes saved.", actions = listOf("dismiss"), kind = "value_item"))
        assertEquals(CardKind.Update, c.kind)
        assertEquals(CardIcon.Check, c.icon)
        assertEquals(CardIcon.Warning, InboxWords.card(item("Disk space is getting low", "", listOf("dismiss"), kind = "value_item")).icon)
    }

    @Test fun agesAndTimesAreReadFromHqsOffsetForm() {
        val now = Instant.parse("2026-10-21T12:05:00Z")
        assertEquals("5 min ago", InboxWords.ago("2026-10-21T12:00:00+00:00", now))
        assertEquals("1 hour ago", InboxWords.ago("2026-10-21T11:00:00+00:00", now))
        assertEquals("yesterday", InboxWords.ago("2026-10-20T10:00:00+00:00", now))
        assertEquals("14:02", InboxWords.clock("2026-10-21T14:02:11.5+00:00", ZoneOffset.UTC))
        assertEquals("", InboxWords.ago("junk", now))
    }

    @Test fun theQuestionIsPickedNotTheChoicesUnderIt() {
        assertEquals("Do you want to edit Chat.dc.html?", InboxWords.lastLine(blocked))
        assertEquals("Proceed", InboxWords.lastLine("Session s on h is blocked on a prompt or approval:\nProceed\nRead it with x"))
    }

    @Test fun aTaskReadyToCheckIsNotOfferedAsAllow() {
        val c = InboxWords.card(item("FR-58: Voice entry: skeleton and first test", "Add **a service** so it can be chosen. See `hq net`.", source = "task_ready_for_review"))
        assertEquals(CardKind.TaskReview, c.kind)
        assertEquals("Voice entry: skeleton and first test", c.title)
        assertEquals("FR-58", c.taskRef)
        assertEquals("Check it", c.allowLabel)
        assertFalse(c.canAllow)
        assertEquals("Add a service so it can be chosen. See hq net.", c.body)
        assertEquals("Task ready to check", c.where)
    }

    @Test fun taskReviewsSitBetweenHelperQuestionsAndOtherApprovals() {
        val pending = listOf(
            item("Delete 3 old backups?", "The nightly backup wants to remove them.", source = "backup"),
            item("FR-1: Check me", "done", source = "task_ready_for_review"),
            item("Harness session 'claude in x' is waiting for you", blocked, source = "session-supervisor"),
            item("Backup finished", "ok", actions = emptyList(), kind = "fyi"),
        )
        val lists = inboxLists(pending, emptyList())
        assertEquals(listOf(CardKind.HelperAsks, CardKind.TaskReview, CardKind.Approval), lists.needs.map { it.kind })
        assertEquals(1, lists.updates.size)
    }

    @Test fun anUpdateShowsNoBackticksOrStars() {
        val c = InboxWords.card(item("Skills improved: x", "Undo any change with `hq skills revert <name>`. **Done.**", actions = emptyList(), kind = "fyi"))
        assertEquals("Undo any change with hq skills revert <name>. Done.", c.body)
    }

    @Test fun aDayIsNamedTodayYesterdayOrByDate() {
        val now = Instant.parse("2026-10-21T12:00:00Z")
        val utc = ZoneOffset.UTC
        assertEquals("Today", InboxWords.dayLabel("2026-10-21T01:00:00+00:00", now, utc))
        assertEquals("Yesterday", InboxWords.dayLabel("2026-10-20T23:00:00+00:00", now, utc))
        assertEquals(java.time.LocalDate.of(2026, 10, 17).format(java.time.format.DateTimeFormatter.ofPattern("EEE d MMM", java.util.Locale.getDefault())), InboxWords.dayLabel("2026-10-17T10:00:00+00:00", now, utc))
        assertEquals("Earlier", InboxWords.dayLabel("not a date", now, utc))
    }

    @Test fun whatTheOwnerDecidedIsListedWithTheTaskTitleOnly() {
        val history = listOf(
            // The server sends no actions for an item that is no longer pending.
            InboxItem("h1", "action_needed", "FR-278: Phase 5: game layer", "", "dismissed", "2026-10-21T12:00:00+00:00", null, "task_ready_for_review", null, emptyList()),
        )
        assertEquals("Not now: Phase 5: game layer", inboxLists(emptyList(), history).done.single().title)
    }
}
