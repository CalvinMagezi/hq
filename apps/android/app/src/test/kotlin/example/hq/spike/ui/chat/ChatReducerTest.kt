package example.hq.spike.ui.chat

import example.hq.spike.data.ChatEvent
import example.hq.spike.data.StoredMessage
import example.hq.spike.data.ToolStep
import java.time.Instant
import java.time.ZoneOffset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatReducerTest {
    private val saved = StoredMessage("m1", "t1", "user", "hello", "x", emptyList(), false)

    @Test
    fun myQuestionIsSwappedForTheSavedOneAndTheChatLearnsItsId() {
        val s = ChatUiState(messages = listOf(UiMessage("c1", Speaker.User, "hello", sending = true)))
        val out = ChatReducer.apply(s, ChatEvent.TurnStart("t1", "c1", saved), setOf("c1"))
        assertEquals("t1", out.threadId)
        assertEquals(listOf(UiMessage("m1", Speaker.User, "hello")), out.messages)
        assertTrue(out.running)
    }

    @Test
    fun otherChatsAndOtherPeoplesQuestionsAreIgnoredOrAdded() {
        val open = ChatUiState(threadId = "t1")
        assertEquals(open, ChatReducer.apply(open, ChatEvent.Text("t2", "x"), emptySet()))
        val fromAnotherTab = ChatReducer.apply(open, ChatEvent.TurnStart("t1", "other", saved), emptySet())
        assertEquals(listOf(UiMessage("m1", Speaker.User, "hello")), fromAnotherTab.messages)
    }

    @Test
    fun aReplyStreamsThroughStepsAndEndsAsOneMessage() {
        var s = ChatReducer.apply(ChatUiState(threadId = "t1"), ChatEvent.TurnStart("t1", null, saved), emptySet())
        s = ChatReducer.apply(s, ChatEvent.ToolStarted("t1", ToolStep("c1", "task_list", "{}", null, null)), emptySet())
        assertEquals("Checked your tasks...", s.live?.headline)
        s = ChatReducer.apply(s, ChatEvent.ToolFinished("t1", "c1", "2 tasks"), emptySet())
        s = ChatReducer.apply(s, ChatEvent.Text("t1", "Two "), emptySet())
        s = ChatReducer.apply(s, ChatEvent.Text("t1", "tasks."), emptySet())
        s = ChatReducer.apply(s, ChatEvent.TurnEnd("t1", "m2", false), emptySet())
        assertNull(s.live)
        val reply = s.messages.last()
        assertEquals("Two tasks.", reply.text)
        assertEquals(1, reply.tools.size)
        assertFalse(s.failed)
    }

    @Test
    fun aTurnThatEndsWithNothingMarksTheReplyFailedUnlessStopped() {
        val running = ChatUiState(threadId = "t1", live = LiveReply())
        assertTrue(ChatReducer.apply(running, ChatEvent.TurnEnd("t1", null, false), emptySet()).failed)
        assertFalse(ChatReducer.apply(running, ChatEvent.TurnEnd("t1", null, true), emptySet()).failed)
    }

    @Test
    fun aMessagePostedByTheServerIsNotAFailedReply() {
        val idle = ChatUiState(threadId = "t1", messages = listOf(UiMessage("u1", Speaker.User, "hi")))
        val out = ChatReducer.apply(idle, ChatEvent.TurnEnd("t1", "m9", false), emptySet())
        assertEquals(idle, out)
    }

    @Test
    fun anEditElsewhereReplacesTheQuestionAndWhatFollowedIt() {
        val open = ChatUiState(
            threadId = "t1",
            messages = listOf(UiMessage("u1", Speaker.User, "first"), UiMessage("r1", Speaker.Reply, "answer"), UiMessage("u2", Speaker.User, "second"), UiMessage("r2", Speaker.Reply, "again")),
        )
        val saved = StoredMessage("u3", "t1", "user", "second, edited", "", emptyList(), false)
        val out = ChatReducer.apply(open, ChatEvent.TurnStart("t1", "other", saved, replaceFrom = "u2"), emptySet())
        assertEquals(listOf("u1", "r1", "u3"), out.messages.map { it.id })
    }

    @Test
    fun toolWordsAreThePlainOnes() {
        assertEquals("Read \"Mobile-First-Roadmap\"", ToolWords.running(ToolStep("c", "vault_read", "{\"path\":\"Notebooks/Projects/Mobile-First-Roadmap.md\"}", null, null)))
        assertEquals("Did a step", ToolWords.running(ToolStep("c", "something_new", null, null, null)))
    }

    @Test
    fun chatsAreGroupedByDay() {
        val now = Instant.parse("2026-10-21T12:00:00Z")
        assertEquals(Group.Today, ChatTime.group("2026-10-21T01:00:00Z", now, ZoneOffset.UTC))
        assertEquals(Group.Yesterday, ChatTime.group("2026-10-20T23:00:00Z", now, ZoneOffset.UTC))
        assertEquals(Group.Week, ChatTime.group("2026-10-17T10:00:00Z", now, ZoneOffset.UTC))
        assertEquals(Group.Older, ChatTime.group("2026-09-01T10:00:00Z", now, ZoneOffset.UTC))
        assertEquals("3m ago", ChatTime.age("2026-10-21T11:57:00Z", now))
    }

    @Test
    fun timesAsHqWritesThemAreRead() {
        // hq stores Rust's to_rfc3339(): an offset and fractional seconds, never a Z.
        val now = Instant.parse("2026-10-21T12:00:00Z")
        assertEquals(Group.Today, ChatTime.group("2026-10-21T01:00:00.123456+00:00", now, ZoneOffset.UTC))
        assertEquals("3m ago", ChatTime.age("2026-10-21T11:56:59.5+00:00", now))
    }
}
