package example.hq.spike.data

import example.hq.spike.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectionTest {
    private val now = 1_000_000L
    private fun inputs(
        alive: Boolean = true,
        step: Int? = null,
        moved: Long? = null,
        usual: Long? = null,
        offline: Long? = null,
        socketDown: Long? = null,
        ai: Long? = null,
        server: Long? = null,
        signedOut: Boolean = false,
    ) = ConnectionInputs(now, alive, false, step, now - 12_000, moved, usual, signedOut, server, socketDown, offline, "openrouter", ai)

    @Test fun the_first_true_row_wins() {
        assertTrue(ConnectionRules.decide(inputs(alive = false, offline = 0)) is Connection.Stopped)
        assertEquals(Connection.Starting(2, 12, null), ConnectionRules.decide(inputs(step = 2, moved = now - 1_000, offline = 0)))
        assertEquals(Connection.ServerSignedOut, ConnectionRules.decide(inputs(signedOut = true, server = 0)))
        assertTrue(ConnectionRules.decide(inputs(socketDown = now - 1, offline = now - 10_000)) is Connection.Reconnecting)
        assertTrue(ConnectionRules.decide(inputs(offline = now - 10_000, ai = now - 1)) is Connection.Offline)
        assertTrue(ConnectionRules.decide(inputs(ai = now - 1)) is Connection.AiNotAnswering)
        assertEquals(Connection.Connected, ConnectionRules.decide(inputs()))
    }

    @Test fun stopped_is_judged_by_progress_never_a_fixed_timer() {
        assertTrue(ConnectionRules.decide(inputs(step = 1, moved = now - 59_000)) is Connection.Starting)
        assertTrue(ConnectionRules.decide(inputs(step = 1, moved = now - 60_000)) is Connection.Stopped)
        // A phone that usually takes 50 s gets 100 s before HQ counts as stopped.
        assertTrue(ConnectionRules.decide(inputs(step = 1, moved = now - 90_000, usual = 50_000)) is Connection.Starting)
    }

    @Test fun offline_waits_three_seconds_and_the_ai_state_clears_after_ten_minutes() {
        assertEquals(Connection.Connected, ConnectionRules.decide(inputs(offline = now - 2_000)))
        assertEquals(Connection.Connected, ConnectionRules.decide(inputs(ai = now - ConnectionRules.AI_STATE_MS)))
    }

    @Test fun words_and_outcomes_follow_the_catalogue() {
        assertEquals(R.string.state_connected, ConnectionWords.of(Connection.Connected).res)
        assertEquals(ConnectionTone.Working, ConnectionWords.of(Connection.Starting(0, 3, null)).tone)
        assertEquals(R.string.outcome_retries_when_online, ConnectionWords.outcome(Connection.Offline(0), retriesItself = true))
        assertEquals(R.string.outcome_start_hq, ConnectionWords.outcome(Connection.Stopped(0, null), retriesItself = false))
        assertEquals(R.string.outcome_hq_fine, ConnectionWords.outcome(Connection.AiNotAnswering(null, 0), retriesItself = true))
        assertEquals(null, ConnectionWords.reason(Connection.Connected))
    }
}
