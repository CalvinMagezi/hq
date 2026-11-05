package example.hq.spike.safe

import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.util.zip.Deflater
import java.util.zip.ZipFile
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** The streaming writer behind the copy: read back with the JDK, in plain and Zip64 forms (4 GB check without 4 GB). */
class ZipStreamTest {
    @get:Rule val tmp = TemporaryFolder()

    private fun roundTrip(zip64: Boolean): Map<String, ByteArray> {
        val out = ByteArrayOutputStream()
        val z = ZipStream(out, zip64)
        val a = ByteArray(100_000) { (it % 7).toByte() }
        val b = ByteArray(5) { it.toByte() }
        z.add("dir/a.bin", 1_700_000_000_000, Deflater.BEST_SPEED, a.size.toLong(), a.inputStream())
        z.add("b.bin", 1_700_000_000_000, Deflater.NO_COMPRESSION, b.size.toLong(), b.inputStream())
        z.add("empty.txt", 1_700_000_000_000, Deflater.BEST_SPEED, 0, ByteArray(0).inputStream())
        z.close()
        val f = tmp.newFile().also { it.writeBytes(out.toByteArray()) }
        return ZipFile(f).use { zf -> zf.entries().asSequence().associate { it.name to zf.getInputStream(it).readBytes() } }
    }

    @Test fun plainZipReadsBack() {
        val m = roundTrip(false)
        assertEquals(3, m.size)
        assertEquals(100_000, m.getValue("dir/a.bin").size)
        assertArrayEquals(ByteArray(5) { it.toByte() }, m.getValue("b.bin"))
        assertEquals(0, m.getValue("empty.txt").size)
    }

    @Test fun zip64ZipReadsBack() {
        val m = roundTrip(true)
        assertEquals(3, m.size)
        assertEquals(100_000, m.getValue("dir/a.bin").size)
    }

    @Test fun aSourceThatFailsMidFileLeavesAValidZipAndReportsIt() {
        val out = ByteArrayOutputStream()
        val z = ZipStream(out)
        val bad = object : InputStream() {
            var n = 0
            override fun read(): Int = if (n++ < 10) 1 else throw java.io.IOException("gone")
            override fun read(b: ByteArray, off: Int, len: Int): Int = if (n >= 10) throw java.io.IOException("gone") else { n = 10; b.fill(1, off, off + 10); 10 }
        }
        assertFalse(z.add("bad.bin", 0, Deflater.BEST_SPEED, 100, bad))
        assertTrue(z.add("good.txt", 0, Deflater.BEST_SPEED, 2, "ok".byteInputStream()))
        z.close()
        val f = tmp.newFile().also { it.writeBytes(out.toByteArray()) }
        ZipFile(f).use { assertEquals("ok", it.getInputStream(it.getEntry("good.txt")).readBytes().decodeToString()) }
    }

    @Test fun anOutputFailureIsThrown() {
        val dead = object : java.io.OutputStream() { override fun write(b: Int) = throw java.io.IOException("ENOSPC") }
        val r = runCatching { ZipStream(dead).add("a", 0, Deflater.BEST_SPEED, 4, "abcd".byteInputStream()) }
        assertTrue(r.exceptionOrNull() is java.io.IOException)
    }
}
