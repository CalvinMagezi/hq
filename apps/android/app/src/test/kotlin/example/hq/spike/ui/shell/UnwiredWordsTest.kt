package example.hq.spike.ui.shell

import example.hq.spike.R
import example.hq.spike.data.Connection
import example.hq.spike.testing.testWords
import example.hq.spike.ui.tasks.AwayWords
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class UnwiredWordsTest {
    private val w = testWords()

    @Test fun awayFailureFollowsTheConnection() {
        assertEquals(R.string.fail_load_away_starting, AwayWords.loadFailure(Connection.Restarting(3)))
        assertEquals(R.string.fail_load_away_stopped, AwayWords.loadFailure(Connection.Stopped(0, null)))
        assertEquals(R.string.fail_load_away_no_reply, AwayWords.loadFailure(Connection.Connected))
    }

    @Test fun restartingSaysHowLong() {
        val words = example.hq.spike.data.ConnectionWords.of(Connection.Restarting(7))
        assertEquals("Restarting HQ · 7 s", w.s(words.res, *words.args.toTypedArray()))
    }

    @Test fun newCatalogueLinesRead() {
        assertEquals("over 25 MB", w.s(R.string.chat_file_too_big))
        assertEquals("Wi-Fi", w.s(R.string.conn_net_wifi))
        assertTrue(w.s(R.string.server_new_phone_title).isNotBlank())
        assertEquals("None paired", w.s(R.string.moved_none_paired_title))
    }
}
