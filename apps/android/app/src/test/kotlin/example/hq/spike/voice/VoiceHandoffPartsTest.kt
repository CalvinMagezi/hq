package example.hq.spike.voice

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VoiceHandoffPartsTest {
    @Test
    fun theBacklogKeepsTheNewestChunksOldestFirst() {
        val b = AudioBacklog(2)
        b.add(shortArrayOf(1, 2, 3), 2)
        b.add(shortArrayOf(4), 1)
        b.add(shortArrayOf(5), 1)
        assertEquals(listOf(listOf<Short>(4), listOf<Short>(5)), b.drain().map { it.toList() })
        assertTrue(b.drain().isEmpty())
    }

    @Test
    fun aSilencedOrStoppedMicIsBlocked() {
        assertTrue(MicAsk.blocked(recording = false, sdk = 28, silenced = false))
        assertTrue(MicAsk.blocked(recording = true, sdk = 29, silenced = true))
        assertFalse(MicAsk.blocked(recording = true, sdk = 28, silenced = true))
        assertFalse(MicAsk.blocked(recording = true, sdk = 33, silenced = false))
    }
}
