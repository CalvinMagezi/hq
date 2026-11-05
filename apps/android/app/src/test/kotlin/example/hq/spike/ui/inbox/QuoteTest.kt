package example.hq.spike.ui.inbox

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class QuoteTest {
    @Test
    fun aVoiceApprovalSplitsOffWhatWasSaid() {
        val (q, rest) = InboxWords.splitQuote("You said: “Move the file”\n\nHQ wants to run mv.")
        assertEquals("Move the file", q)
        assertEquals("HQ wants to run mv.", rest.trim())
    }

    @Test
    fun anOrdinaryApprovalHasNoQuote() {
        val (q, rest) = InboxWords.splitQuote("HQ wants to run mv.")
        assertNull(q)
        assertEquals("HQ wants to run mv.", rest)
    }
}
