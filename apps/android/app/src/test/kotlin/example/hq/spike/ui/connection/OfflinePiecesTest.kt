package example.hq.spike.ui.connection

import example.hq.spike.R
import example.hq.spike.data.AiStatus
import example.hq.spike.data.Connection
import example.hq.spike.testing.testWords
import java.time.ZoneId
import java.time.ZonedDateTime
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class OfflinePiecesTest {
    private val w = testWords()
    private val noon = ZonedDateTime.of(2026, 11, 3, 12, 0, 0, 0, ZoneId.systemDefault()).toInstant().toEpochMilli()

    @Test fun theStripShowsForEveryStateThatKeepsHqFromAnswering() {
        assertNull(OfflineStrip.lead(Connection.Connected))
        assertNull(OfflineStrip.lead(Connection.Reconnecting(0)))
        assertNull(OfflineStrip.lead(Connection.AiNotAnswering("x", 0)))
        listOf(Connection.Starting(0, 1, null), Connection.Restarting(1), Connection.Stopped(0, null), Connection.Offline(0), Connection.ServerUnreachable(0), Connection.ServerSignedOut)
            .forEach { assertTrue(it.toString(), OfflineStrip.lead(it) != null) }
    }

    @Test fun aCopyFromTodayNamesTheClockAndAnOlderOneTheDay() {
        val today = OfflineStrip.line(w, Connection.Stopped(0, null), noon - 60_000, noon)!!
        assertTrue(today, today.startsWith("HQ has stopped. Saved on this phone at "))
        val older = OfflineStrip.line(w, Connection.Offline(0), noon - 3 * 86_400_000L, noon)!!
        assertTrue(older, older.startsWith("You're offline. Saved on this phone on "))
        assertEquals("HQ is starting.", OfflineStrip.line(w, Connection.Starting(0, 2, null), null, noon))
        assertNull(OfflineStrip.line(w, Connection.Connected, noon, noon))
    }

    @Test fun theChatBannerHasOneLinePerState() {
        assertNull(ChatBanner.words(Connection.Connected))
        assertEquals(R.string.chat_bar_stopped, ChatBanner.words(Connection.Stopped(0, null)))
        assertEquals(R.string.chat_bar_reconnecting, ChatBanner.words(Connection.Reconnecting(0)))
        assertEquals(R.string.chat_bar_ai_down, ChatBanner.words(Connection.AiNotAnswering(null, 0)))
        assertEquals(R.string.chat_bar_signed_out, ChatBanner.words(Connection.ServerSignedOut))
        assertEquals("You're offline. Messages wait here and send when you're back online.", w.s(ChatBanner.words(Connection.Offline(0))!!))
    }

    @Test fun theAiStatusReadsTheNewestFailureAndNeverHoldsText() {
        assertTrue(AiStatus.parse("""{"answering":true,"services":[]}""").answering)
        val down = AiStatus.parse("""{"answering":false,"services":[{"provider":"a","last_failed_at":100,"last_ok_at":50,"failure":"timeout"},{"provider":"b","last_failed_at":200,"last_ok_at":10,"failure":"auth"}]}""")
        assertFalse(down.answering)
        assertEquals("b", down.failingService)
        assertEquals(200_000L, down.failedAtMs)
        assertTrue(AiStatus.parse("not json").answering)
    }
}
