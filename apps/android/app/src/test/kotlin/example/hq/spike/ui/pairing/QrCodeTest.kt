package example.hq.spike.ui.pairing

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The scanner's decoder reads what the screen's encoder draws, at the size of a real pairing link. */
class QrCodeTest {
    private val link = "hqpair://ticket/" + "eyJ2IjozLCJrIjoicGFpciIsImlkIjoiZjEzODM5NWFiOGMwNjQwYjBiZjJkZGMyOTUxNDU2YTUyYzM1MDg1OTJkNjJhMzEzOTIxYWExNDgzZDFmZTJlMCJ9".repeat(3)

    private fun decode(text: String, scale: Int, rowPad: Int): String? {
        val m = QrCode.encode(text)
        val width = m.width * scale
        val height = m.height * scale
        val stride = width + rowPad
        val luma = ByteArray(stride * height) { 0xFF.toByte() }
        for (y in 0 until height) for (x in 0 until width) if (m.get(x / scale, y / scale)) luma[y * stride + x] = 0
        return QrCode.decode(luma, stride, width, height)
    }

    @Test fun aPairingLinkSurvivesTheRoundTrip() = assertEquals(link, decode(link, 4, 0))

    @Test fun cameraRowPaddingDoesNotMatter() = assertEquals(link, decode(link, 3, 24))

    @Test fun aSmallCodeAtALowResolutionStillReads() {
        val short = "hqpair://ticket/eyJ2IjozLCJrIjoicGFpciJ9"
        assertEquals(short, decode(short, 3, 0))
    }

    @Test fun aBlankFrameHasNoCode() {
        val blank = ByteArray(200 * 200) { 0x80.toByte() }
        assertNull(QrCode.decode(blank, 200, 200, 200))
    }
}
