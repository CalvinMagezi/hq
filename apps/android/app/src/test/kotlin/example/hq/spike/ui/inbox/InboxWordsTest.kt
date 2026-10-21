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
        assertTrue(c.body.startsWith("It stopped to ask:"))
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
}
