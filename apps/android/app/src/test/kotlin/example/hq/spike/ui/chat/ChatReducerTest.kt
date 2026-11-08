package example.hq.spike.ui.chat

import example.hq.spike.R
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
    fun aMessageThatIsAlreadyShownIsNotAddedTwiceWhenItsTurnEnds() {
        // The chat was reloaded after a lag and already holds the saved reply.
        val reloaded = ChatUiState(threadId = "t1", live = LiveReply(text = "Done."), messages = listOf(UiMessage("m1", Speaker.User, "hello"), UiMessage("r1", Speaker.Reply, "Done.")))
        val out = ChatReducer.apply(reloaded, ChatEvent.TurnEnd("t1", "r1", false), emptySet())
        assertEquals(listOf("m1", "r1"), out.messages.map { it.id })
        assertNull(out.live)
    }

    @Test
    fun theSavedQuestionAppearsOnceWhenTheChatWasReloadedBeforeItsTurnStarted() {
        val s = ChatUiState(messages = listOf(UiMessage("m1", Speaker.User, "hello"), UiMessage("c1", Speaker.User, "hello", sending = true)))
        val out = ChatReducer.apply(s, ChatEvent.TurnStart("t1", "c1", saved), setOf("c1"))
        assertEquals(listOf("m1"), out.messages.map { it.id })
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
        assertEquals(StepLine(R.string.step_tasks_checked), s.live?.headline)
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

    private fun step(name: String, args: String? = null) = ToolStep("c", name, args, null, null)

    @Test
    fun toolWordsNameWhatAStepTouched() {
        assertEquals(StepLine(R.string.step_note_read_named, "Mobile-First-Roadmap"), ToolWords.line(step("vault_read", "{\"path\":\"Notebooks/Projects/Mobile-First-Roadmap.md\"}")))
        assertEquals(StepLine(R.string.step_web_search_for, "UK public holidays"), ToolWords.line(step("web_search", "{\"query\":\"UK public holidays\"}")))
        assertEquals(StepLine(R.string.step_web_fetch_site, "timeanddate.com"), ToolWords.line(step("web_fetch", "{\"url\":\"https://www.timeanddate.com/holidays/uk/\"}")))
        assertEquals(StepLine(R.string.step_task_added_named, "Call the bank"), ToolWords.line(step("task_create", "{\"title\":\"Call the bank\"}")))
    }

    @Test
    fun toolWordsWithoutANameStillSayWhatKindOfThing() {
        assertEquals(StepLine(R.string.step_web_search), ToolWords.line(step("web_search")))
        assertEquals(StepLine(R.string.step_web_fetch), ToolWords.line(step("web_fetch", "{\"url\":\"not a url\"}")))
        assertEquals(StepLine(R.string.step_tasks_checked), ToolWords.line(step("task_list", "{}")))
        assertEquals(StepLine(R.string.step_command_ran), ToolWords.line(step("bash", "not json")))
    }

    @Test
    fun anUnknownToolFallsBackByFamilyThenItsOwnNameNeverDidAStep() {
        assertEquals(StepLine(R.string.step_notes_worked), ToolWords.line(step("vault_something_new")))
        assertEquals(StepLine(R.string.step_helper_checked), ToolWords.line(step("harness_session_peek")))
        assertEquals(StepLine(R.string.step_used, "system clock"), ToolWords.line(step("system_clock")))
    }

    @Test
    fun theGatewayCallIsReadAsTheToolItCalls() {
        val call = step("hq_call", "{\"tool\":\"web_search\",\"args\":{\"query\":\"rain in Kampala\"}}")
        assertEquals(StepLine(R.string.step_web_search_for, "rain in Kampala"), ToolWords.line(call))
    }

    @Test
    fun longNamesAreCutToFitAPhone() {
        val arg = ToolWords.line(step("web_search", "{\"query\":\"${"a".repeat(200)}\"}")).arg!!
        assertTrue(arg.length <= 60 && arg.endsWith("…"))
    }

    @Test
    fun aRunOfTheSameLineIsSaidOnce() {
        val lines = ToolWords.lines(
            listOf(step("task_list"), step("task_list"), step("web_search", "{\"query\":\"a\"}"), step("web_search", "{\"query\":\"b\"}"), step("task_list")),
        )
        assertEquals(
            listOf(StepLine(R.string.step_tasks_checked), StepLine(R.string.step_web_search_for, "a"), StepLine(R.string.step_web_search_for, "b"), StepLine(R.string.step_tasks_checked)),
            lines,
        )
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
