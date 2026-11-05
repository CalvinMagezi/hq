package example.hq.spike.voice

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.R
import example.hq.spike.data.Connection
import example.hq.spike.data.HqServices
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowSystemClock
import java.time.Duration

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class VoiceSessionTest {
    private val app: Context = ApplicationProvider.getApplicationContext()
    private val scope = CoroutineScope(Dispatchers.Unconfined)
    private val stored = StringBuilder()
    private val noticed = mutableListOf<String>()
    private lateinit var session: VoiceSession
    private lateinit var handoff: VoiceHandoff

    private val store = object : HandoffStore {
        override fun load() = stored.toString().ifEmpty { null }
        override fun save(text: String) { stored.clear(); stored.append(text) }
    }
    private val api = object : HandoffApi {
        override suspend fun running() = setOf("t1")
        override suspend fun threadOf(clientId: String): String? = null
        override suspend fun lastReply(threadId: String): String? = null
    }
    private val notices = object : HandoffNotices {
        override fun answered(threadId: String, answer: String?) = false.also { noticed += "answered" }
        override fun needsOk(threadId: String) = false.also { noticed += "needsOk" }
        override fun failed(threadId: String?, reason: String) = false.also { noticed += "failed" }
    }

    @Before fun open() {
        ShadowSystemClock.advanceBy(Duration.ofMinutes(5))
        handoff = VoiceHandoff(MutableSharedFlow(), MutableStateFlow(true), api, store, notices, scope, firstPollMs = 600_000L)
        session = VoiceSession(app, scope, HqServices.get(app).connection, MutableStateFlow<Connection>(Connection.Connected), handoff)
    }

    @After fun close() = scope.cancel()

    private fun say(id: Int) = app.getString(id)

    @Test fun aNewSheetIsIdleWithNothingHeardOrShown() {
        val s = session.state.value
        assertEquals(VoicePhase.Idle, s.phase)
        assertEquals("", s.heard)
        assertEquals("", s.reply)
        assertNull(s.problem)
        assertFalse(s.needsOk)
    }

    @Test fun theTurnsOwnFailureCodesBecomeTheCataloguesSentences() {
        val cases = mapOf(
            VoiceTurn.HQ_STOPPED to R.string.voice_fail_hq_stopped,
            VoiceTurn.RESTARTING to R.string.voice_fail_restarting,
            VoiceTurn.NO_REPLY to R.string.voice_fail_no_reply,
            VoiceTurn.STILL_WORKING to R.string.voice_still_working,
            VoiceTurn.LOST_PART to R.string.voice_fail_lost_part,
            VoiceTurn.BUSY to R.string.voice_fail_busy,
            VoiceTurn.NOT_ACCEPTED to R.string.voice_fail_not_accepted,
        )
        cases.forEach { (code, line) ->
            session.onOutput(TurnOutput.Failed(code))
            assertEquals(say(line), session.state.value.problem)
        }
    }

    @Test fun hqsOwnReasonIsShownAsItCame() {
        session.onOutput(TurnOutput.Failed("The AI service isn't answering."))
        assertEquals("The AI service isn't answering.", session.state.value.problem)
    }

    @Test fun theReplyGrowsAsItStreamsAndAResetReplacesIt() {
        session.onOutput(TurnOutput.Thread("t1"))
        session.onOutput(TurnOutput.Text("Two "))
        session.onOutput(TurnOutput.Text("tasks."))
        assertEquals("Two tasks.", session.state.value.reply)
        assertEquals(VoicePhase.Answering, session.state.value.phase)
        assertEquals("t1", session.state.value.threadId)
        session.onOutput(TurnOutput.Reset("Two tasks are due."))
        assertEquals("Two tasks are due.", session.state.value.reply)
    }

    @Test fun aLookupShowsLookingUpAndAnOkRequestIsNotAnError() {
        session.onOutput(TurnOutput.Working)
        assertEquals(VoicePhase.LookingUp, session.state.value.phase)
        session.onOutput(TurnOutput.NeedsOk)
        assertTrue(session.state.value.needsOk)
        assertNull(session.state.value.problem)
    }

    @Test fun aRefusalGoesBackToIdleWithItsLine() {
        session.refuse("Nope.")
        assertEquals(VoicePhase.Idle, session.state.value.phase)
        assertEquals("Nope.", session.state.value.problem)
    }

    @Test fun microphoneRefusalsKeepTheirPhasesAndTheStillOffLine() {
        session.micNeeded()
        assertEquals(VoicePhase.MicNeeded, session.state.value.phase)
        session.micBlocked()
        assertEquals(VoicePhase.MicBlocked, session.state.value.phase)
        assertNull(session.state.value.problem)
        session.micBlocked(still = true)
        assertEquals(say(R.string.voice_mic_still_off), session.state.value.problem)
    }

    @Test fun dontSendDropsTheQuestionAndKeepsTheWords() {
        session.refuse("x")
        session.dontSend()
        assertEquals(VoicePhase.Idle, session.state.value.phase)
        assertNull(session.state.value.problem)
    }

    @Test fun sendAgainWithNothingHeardDoesNothing() {
        session.sendAgain()
        assertEquals(VoicePhase.Idle, session.state.value.phase)
    }

    @Test fun withoutAModelTheMicDoesNothing() {
        session.listen()
        assertTrue(session.state.value.phase == VoicePhase.NeedsModel || session.state.value.phase == VoicePhase.Downloading)
        session.micPressed()
        assertTrue(session.state.value.phase == VoicePhase.NeedsModel || session.state.value.phase == VoicePhase.Downloading)
    }

    @Test fun closingWhileHqWorksHandsTheQuestionToTheHandoff() {
        session.onOutput(TurnOutput.Thread("t1"))
        session.onOutput(TurnOutput.Working)
        ShadowSystemClock.advanceBy(Duration.ofSeconds(10))
        session.quiet()
        assertEquals(VoicePhase.Idle, session.state.value.phase)
        assertTrue(stored.contains("\"thread\":\"t1\""))
    }

    @Test fun closingWithNothingRunningHandsOverNothing() {
        session.quiet()
        assertTrue(stored.isEmpty())
    }

    @Test fun openingTheChatClearsTheNextOpenLine() {
        stored.append("""{"last":{"thread":"t9","at":${System.currentTimeMillis()},"posted":false}}""")
        assertEquals("t9", handoff.unseenAnswer())
        session.chatOpened()
        assertNull(handoff.unseenAnswer())
    }
}
