package example.hq.spike

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ServerHqTest {
    @Test fun anAddressIsHttpsWithAHostAndNothingElse() {
        assertEquals("https://hq.example.com", ServerHq.normalize("hq.example.com"))
        assertEquals("https://hq.example.com", ServerHq.normalize(" https://HQ.example.com/ "))
        assertEquals("https://hq.example.ts.net:8443", ServerHq.normalize("https://hq.example.ts.net:8443"))
    }

    @Test fun anAddressThatWouldSendTheTokenInTheClearOrSomewhereElseIsRefused() {
        assertNull("never in the clear", ServerHq.normalize("http://hq.example.com"))
        assertNull("HQ is served at the root", ServerHq.normalize("https://hq.example.com/notifications"))
        assertNull("a pasted login link carries a token in its fragment", ServerHq.normalize("https://hq.example.com/#token=abc"))
        assertNull(ServerHq.normalize("https://hq.example.com?x=1"))
        assertNull(ServerHq.normalize("https://someone:secret@hq.example.com"))
        assertNull(ServerHq.normalize("hq example.com"))
        assertNull(ServerHq.normalize("   "))
    }

    @Test fun anItemFromTheServerCarriesAPrefixItsAnswerTakesOff() {
        val id = ServerHq.ID_PREFIX + "val_abc"
        assertTrue(ServerHq.isServer(id))
        assertEquals("val_abc", ServerHq.rawId(id))
        assertFalse(ServerHq.isServer("val_abc"))
        assertEquals("val_abc", ServerHq.rawId("val_abc"))
    }
}
