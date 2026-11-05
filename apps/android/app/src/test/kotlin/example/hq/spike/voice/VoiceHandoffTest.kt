package example.hq.spike.voice

import example.hq.spike.data.ChatEvent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class VoiceHandoffTest {
    private class FakeApi(var running: Set<String> = emptySet(), var thread: String? = null, var reply: String? = "It is done.") : HandoffApi {
        override suspend fun running() = running
        override suspend fun threadOf(clientId: String) = thread
        override suspend fun lastReply(threadId: String) = reply
    }

    private class FakeStore(var text: String? = null) : HandoffStore {
        override fun load() = text
        override fun save(text: String) { this.text = text }
    }

    private class FakeNotices(val posts: Boolean = true) : HandoffNotices {
        val log = mutableListOf<String>()
        override fun answered(threadId: String, answer: String?): Boolean { log += "answered:$threadId:$answer"; return posts }
        override fun needsOk(threadId: String): Boolean { log += "needsOk:$threadId"; return posts }
        override fun failed(threadId: String?, reason: String): Boolean { log += "failed:$threadId:$reason"; return posts }
    }

    private val scope = CoroutineScope(Dispatchers.Default)
    private val events = MutableSharedFlow<ChatEvent>(extraBufferCapacity = 64)
    private val connected = MutableStateFlow(true)
    private val api = FakeApi()
    private val store = FakeStore()
    private val notices = FakeNotices()
    private var clock = 1_000L

    @After fun tearDown() = scope.cancel()

    private fun handoff(n: HandoffNotices = notices, pollMs: Long = 60_000L, firstPollMs: Long = 60_000L, capMs: Long = 600_000L) =
        VoiceHandoff(events, connected, api, store, n, scope, { clock }, pollMs, firstPollMs, capMs)

    private suspend fun until(what: String, check: () -> Boolean) = withTimeout(5_000) {
        while (!check()) delay(10)
    }.also { assertTrue(what, check()) }

    private suspend fun emitWhenWatching(e: ChatEvent) {
        // The watcher subscribes on another thread; a shared flow drops events nobody is collecting yet.
        withTimeout(5_000) { while (events.subscriptionCount.value == 0) delay(10) }
        events.emit(e)
    }

    @Test fun anAnswerAfterTheSheetClosedPostsOneNoticeAndClearsThePendingQuestion() = runBlocking {
        handoff().adopt(Pending("c1", "t1", clock))
        assertTrue(store.text!!.contains("c1"))
        emitWhenWatching(ChatEvent.TurnEnd("t1", "m1", false))
        until("noticed") { notices.log.isNotEmpty() }
        assertEquals(listOf("answered:t1:It is done."), notices.log)
        assertFalse(store.text!!.contains("\"client\":\"c1\""))
    }

    @Test fun anAnswerThatNeedsTheOwnerSaysSoAndIsNotRecordedAsAnswered() = runBlocking {
        val h = handoff()
        h.adopt(Pending("c2", "t2", clock))
        emitWhenWatching(ChatEvent.TurnEnd("t2", null, false, waitingForOwner = true))
        until("noticed") { notices.log.isNotEmpty() }
        assertEquals(listOf("needsOk:t2"), notices.log)
        assertNull(h.unseenAnswer())
    }

    @Test fun aFailureIsReportedWithHqsOwnReason() = runBlocking {
        handoff().adopt(Pending("c3", "t3", clock))
        emitWhenWatching(ChatEvent.Failed("t3", "The AI service isn't answering."))
        until("noticed") { notices.log.isNotEmpty() }
        assertEquals(listOf("failed:t3:The AI service isn't answering."), notices.log)
    }

    @Test fun anotherThreadsEventsAreIgnored() = runBlocking {
        handoff().adopt(Pending("c4", "t4", clock))
        emitWhenWatching(ChatEvent.TurnEnd("other", "m", false))
        delay(200)
        assertTrue(notices.log.isEmpty())
    }

    @Test fun aQuestionAdoptedTwiceIsWatchedOnce() = runBlocking {
        val h = handoff()
        h.adopt(Pending("c5", "t5", clock))
        h.adopt(Pending("c5", "t5", clock))
        emitWhenWatching(ChatEvent.TurnEnd("t5", "m", false))
        until("noticed") { notices.log.isNotEmpty() }
        delay(200)
        assertEquals(1, notices.log.size)
    }

    @Test fun aThreadThatStoppedRunningCountsAsAnsweredOnTheFirstPoll() = runBlocking {
        api.running = emptySet()
        handoff(firstPollMs = 20, pollMs = 20).adopt(Pending("c6", "t6", clock))
        until("polled") { notices.log.isNotEmpty() }
        assertEquals("answered:t6:It is done.", notices.log.single())
    }

    @Test fun aThreadStillRunningIsNotGivenUpOnByPollingAlone() = runBlocking {
        api.running = setOf("t7")
        handoff(firstPollMs = 20, pollMs = 20).adopt(Pending("c7", "t7", clock))
        delay(300)
        assertTrue(notices.log.isEmpty())
    }

    @Test fun theThreadIsFoundByClientIdWhenTheSheetClosedBeforeHqNamedIt() = runBlocking {
        api.thread = "t8"
        handoff(firstPollMs = 20, pollMs = 20).adopt(Pending("c8", null, clock))
        until("found") { notices.log.isNotEmpty() }
        assertEquals("answered:t8:It is done.", notices.log.single())
    }

    @Test fun silenceForTheWholeCapEndsWithNoNoticeAndNothingKept() = runBlocking {
        api.running = setOf("t9")
        handoff(capMs = 100, firstPollMs = 10_000).adopt(Pending("c9", "t9", clock))
        until("dropped") { !store.text!!.contains("\"client\":\"c9\"") }
        assertTrue(notices.log.isEmpty())
    }

    @Test fun anAnswerWithNoNoticePostedIsOfferedOnNextOpenOnceWithinAnHour() = runBlocking {
        val quiet = FakeNotices(posts = false)
        val h = handoff(quiet)
        h.adopt(Pending("c10", "t10", clock))
        emitWhenWatching(ChatEvent.TurnEnd("t10", "m", false))
        until("noticed") { quiet.log.isNotEmpty() }
        assertEquals("t10", h.unseenAnswer())
        clock += VoiceHandoff.UNSEEN_MS + 1
        assertNull(h.unseenAnswer())
        clock -= VoiceHandoff.UNSEEN_MS + 1
        h.markSeen()
        assertNull(h.unseenAnswer())
    }

    @Test fun anAnswerThatWasNoticedIsNotOfferedAgainOnNextOpen() = runBlocking {
        val h = handoff()
        h.adopt(Pending("c11", "t11", clock))
        emitWhenWatching(ChatEvent.TurnEnd("t11", "m", false))
        until("noticed") { notices.log.isNotEmpty() }
        assertNull(h.unseenAnswer())
    }

    @Test fun aQuestionLeftOpenByADeadProcessGetsAWatcherAgain() = runBlocking {
        store.text = """{"pending":[{"client":"c12","thread":"t12","at":$clock}]}"""
        handoff().restore()
        emitWhenWatching(ChatEvent.TurnEnd("t12", "m", false))
        until("noticed") { notices.log.isNotEmpty() }
        assertEquals("answered:t12:It is done.", notices.log.single())
    }

    @Test fun aQuestionOlderThanTheCapIsNotRestored() = runBlocking {
        store.text = """{"pending":[{"client":"c13","thread":"t13","at":${clock - 700_000L}}]}"""
        handoff().restore()
        delay(200)
        events.emit(ChatEvent.TurnEnd("t13", "m", false))
        delay(200)
        assertTrue(notices.log.isEmpty())
    }
}
