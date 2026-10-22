package example.hq.spike.ui.connect

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectWordsTest {
    private val now = 1_000_000L
    private val hub = ConnectFacts(enabled = true, url = "https://abc.example.com", source = UrlSource.Hub)

    @Test fun armingNeedsTheConnectorOnInHqNotAStandIn() {
        assertTrue(ConnectWords.canArm(hub))
        assertFalse(ConnectWords.canArm(ConnectFacts(url = "https://abc.example.com", source = UrlSource.Stub)))
        assertFalse(ConnectWords.canArm(ConnectFacts()))
    }

    @Test fun theWindowIsOpenOnlyBeforeItsEnd() {
        assertTrue(ConnectWords.armed(hub.copy(armedUntil = now + 5), now))
        assertFalse(ConnectWords.armed(hub.copy(armedUntil = now), now))
        assertFalse(ConnectWords.armed(hub, now))
    }

    @Test fun theStatusSaysWhatToDoNext() {
        assertEquals("Off. Nothing can connect.", ConnectWords.status(ConnectFacts(), now))
        assertEquals("On, and nothing is connected. Arm it to let Claude in.", ConnectWords.status(hub, now))
        assertEquals("On. One client is connected.", ConnectWords.status(hub.copy(clients = 1), now))
        assertEquals("On. 3 clients are connected.", ConnectWords.status(hub.copy(clients = 3), now))
        assertEquals("Armed for 9:41 more. Press Connect in Claude now.", ConnectWords.status(hub.copy(armedUntil = now + 581), now))
    }

    @Test fun theIntroAdmitsAStandInAndAMissingAddress() {
        assertTrue(ConnectWords.intro(ConnectFacts()).contains("edge"))
        assertTrue(ConnectWords.intro(ConnectFacts(url = "https://x.example.com", source = UrlSource.Stub)).contains("stand-in"))
        assertTrue(ConnectWords.intro(hub).startsWith("Add this address"))
    }

    @Test fun theStandInFileHoldsOneHttpsAddressOrNothing() {
        assertEquals("https://abc.example.com", ConnectWords.standIn("https://abc.example.com\nignored"))
        assertNull(ConnectWords.standIn("http://abc.example.com"))
        assertNull(ConnectWords.standIn("https://has space.example.com"))
        assertNull(ConnectWords.standIn("https://x"))
        assertNull(ConnectWords.standIn(null))
        assertNull(ConnectWords.standIn(""))
    }
}
