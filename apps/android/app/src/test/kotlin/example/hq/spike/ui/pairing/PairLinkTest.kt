package example.hq.spike.ui.pairing

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PairLinkTest {
    private val good = "hqpair://ticket/eyJ2IjozLCJrIjoicGFpciJ9"

    @Test fun ourLinkIsAccepted() = assertEquals(good, PairLink.accept(good))

    @Test fun theSchemeAndSpacesDoNotMatter() {
        assertEquals(good.replaceFirst("hqpair", "HQPAIR"), PairLink.accept("  " + good.replaceFirst("hqpair", "HQPAIR") + "\n"))
    }

    @Test fun otherSchemesHostsAndBodiesAreNotOurs() {
        assertNull(PairLink.accept(null))
        assertNull(PairLink.accept(""))
        assertNull(PairLink.accept("https://example.com/ticket/eyJ2IjozLCJrIjoicGFpciJ9"))
        assertNull(PairLink.accept("hq://ticket/eyJ2IjozLCJrIjoicGFpciJ9"))
        assertNull(PairLink.accept("hqpair://other/eyJ2IjozLCJrIjoicGFpciJ9"))
        assertNull(PairLink.accept("hqpair://ticket/"))
        assertNull(PairLink.accept("hqpair://ticket/short"))
    }

    @Test fun aBodyWithOddCharactersIsRefusedBeforeItReachesTheBridge() {
        assertNull(PairLink.accept("hqpair://ticket/eyJ2IjozLCJrIjoicGFpciJ9 --relay x"))
        assertNull(PairLink.accept("hqpair://ticket/eyJ2IjozLCJrIjoicGFpciJ9\"}"))
        assertNull(PairLink.accept("hqpair://ticket/eyJ2IjozLCJrIjoicéGFpciJ9"))
    }

    @Test fun aHugeLinkIsRefused() {
        assertNull(PairLink.accept("hqpair://ticket/" + "a".repeat(PairLink.MAX_CHARS)))
    }

    @Test fun theShortIdIsEightCharacters() = assertEquals("66902dde", PairLink.shortId("66902ddec14cff33"))
}
