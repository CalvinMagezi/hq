package example.hq.spike.safe

import java.io.Closeable
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.time.Instant
import java.time.ZoneId
import java.util.zip.CRC32
import java.util.zip.Deflater

/**
 * A streaming .zip writer with Zip64, because Android's own ZipOutputStream cannot write past 4 GB or 65,535 entries on every
 * version this app supports. Entries are deflated as they go and described after their data, so nothing is read twice.
 * Plain Kotlin and the JDK only: the safe-mode process loads it with nothing else of the app.
 */
class ZipStream(out: OutputStream, private val forceZip64: Boolean = false) : Closeable {
    private class Counting(private val out: OutputStream) : OutputStream() {
        var count = 0L
        override fun write(b: Int) { out.write(b); count++ }
        override fun write(b: ByteArray, off: Int, len: Int) { out.write(b, off, len); count += len }
        override fun flush() = out.flush()
        override fun close() = out.close()
    }

    private class Done(val name: ByteArray, val crc: Long, val compressed: Long, val size: Long, val offset: Long, val zip64: Boolean, val dosTime: Int, val dosDate: Int)

    private val sink = Counting(out)
    private val done = ArrayList<Done>()

    /** Bytes written to the output so far. */
    val bytesWritten: Long get() = sink.count

    /**
     * Writes one entry from [source]. [level] is [Deflater.BEST_SPEED] for text and [Deflater.NO_COMPRESSION] for pictures and
     * PDFs, which are already packed. [expected] is the size the file had when planned and decides whether the entry needs Zip64.
     * [onBytes] hears each chunk read. A source that fails while being read ends the entry short and returns false; the .zip
     * stays valid. A failure of the output is thrown.
     */
    fun add(name: String, modified: Long, level: Int, expected: Long, source: InputStream, onBytes: (Int) -> Unit = {}): Boolean {
        val nameBytes = name.toByteArray(Charsets.UTF_8)
        val zip64 = forceZip64 || expected >= ZIP64_ENTRY
        val (dosTime, dosDate) = dos(modified)
        val offset = sink.count
        header(nameBytes, zip64, dosTime, dosDate)

        val crc = CRC32()
        val deflater = Deflater(level, true)
        val buffer = ByteArray(BUFFER)
        val packed = ByteArray(BUFFER)
        var size = 0L
        var complete = true
        val limit = if (zip64) Long.MAX_VALUE else MAX_32
        try {
            while (size < limit) {
                val n = try {
                    source.read(buffer, 0, minOf(buffer.size.toLong(), limit - size).toInt())
                } catch (_: IOException) {
                    complete = false
                    -1
                }
                if (n < 0) break
                if (n == 0) continue
                crc.update(buffer, 0, n)
                size += n
                deflater.setInput(buffer, 0, n)
                while (!deflater.needsInput()) drain(deflater, packed)
                onBytes(n)
            }
            deflater.finish()
            while (!deflater.finished()) drain(deflater, packed)
            val compressed = deflater.bytesWritten
            descriptor(crc.value, compressed, size, zip64)
            done += Done(nameBytes, crc.value, compressed, size, offset, zip64, dosTime, dosDate)
        } finally {
            deflater.end()
        }
        return complete
    }

    /** Writes the index and closes the output. */
    override fun close() {
        val start = sink.count
        done.forEach(::central)
        val size = sink.count - start
        val needs64 = forceZip64 || done.size >= MAX_16 || start >= MAX_32 || size >= MAX_32
        if (needs64) {
            val at = sink.count
            put(ByteBuffer.allocate(56).order(ByteOrder.LITTLE_ENDIAN).apply {
                putInt(0x06064b50); putLong(44); putShort(VERSION_ZIP64.toShort()); putShort(VERSION_ZIP64.toShort())
                putInt(0); putInt(0); putLong(done.size.toLong()); putLong(done.size.toLong()); putLong(size); putLong(start)
            })
            put(ByteBuffer.allocate(20).order(ByteOrder.LITTLE_ENDIAN).apply { putInt(0x07064b50); putInt(0); putLong(at); putInt(1) })
        }
        put(ByteBuffer.allocate(22).order(ByteOrder.LITTLE_ENDIAN).apply {
            putInt(0x06054b50); putShort(0); putShort(0)
            val count = if (needs64) MAX_16 else done.size
            putShort(count.toShort()); putShort(count.toShort())
            putInt((if (needs64) MAX_32 else size).toInt()); putInt((if (needs64) MAX_32 else start).toInt()); putShort(0)
        })
        sink.flush()
        sink.close()
    }

    private fun drain(deflater: Deflater, packed: ByteArray) {
        val c = deflater.deflate(packed)
        if (c > 0) sink.write(packed, 0, c)
    }

    private fun header(name: ByteArray, zip64: Boolean, dosTime: Int, dosDate: Int) {
        val extra = if (zip64) 20 else 0
        put(ByteBuffer.allocate(30 + name.size + extra).order(ByteOrder.LITTLE_ENDIAN).apply {
            putInt(0x04034b50)
            putShort((if (zip64) VERSION_ZIP64 else VERSION_PLAIN).toShort())
            putShort(FLAGS.toShort()); putShort(METHOD_DEFLATED.toShort())
            putShort(dosTime.toShort()); putShort(dosDate.toShort())
            putInt(0)
            putInt(if (zip64) MAX_32.toInt() else 0); putInt(if (zip64) MAX_32.toInt() else 0)
            putShort(name.size.toShort()); putShort(extra.toShort())
            put(name)
            if (zip64) { putShort(1); putShort(16); putLong(0); putLong(0) }
        })
    }

    private fun descriptor(crc: Long, compressed: Long, size: Long, zip64: Boolean) {
        put(ByteBuffer.allocate(if (zip64) 24 else 16).order(ByteOrder.LITTLE_ENDIAN).apply {
            putInt(0x08074b50); putInt(crc.toInt())
            if (zip64) { putLong(compressed); putLong(size) } else { putInt(compressed.toInt()); putInt(size.toInt()) }
        })
    }

    private fun central(e: Done) {
        val bigOffset = forceZip64 || e.offset >= MAX_32
        val extraLen = (if (e.zip64) 16 else 0) + (if (bigOffset) 8 else 0)
        put(ByteBuffer.allocate(46 + e.name.size + (if (extraLen > 0) extraLen + 4 else 0)).order(ByteOrder.LITTLE_ENDIAN).apply {
            putInt(0x02014b50)
            val version = if (e.zip64 || bigOffset) VERSION_ZIP64 else VERSION_PLAIN
            putShort(version.toShort()); putShort(version.toShort())
            putShort(FLAGS.toShort()); putShort(METHOD_DEFLATED.toShort())
            putShort(e.dosTime.toShort()); putShort(e.dosDate.toShort())
            putInt(e.crc.toInt())
            putInt(if (e.zip64) MAX_32.toInt() else e.compressed.toInt()); putInt(if (e.zip64) MAX_32.toInt() else e.size.toInt())
            putShort(e.name.size.toShort())
            putShort((if (extraLen > 0) extraLen + 4 else 0).toShort())
            putShort(0); putShort(0); putShort(0); putInt(0)
            putInt(if (bigOffset) MAX_32.toInt() else e.offset.toInt())
            put(e.name)
            if (extraLen > 0) {
                putShort(1); putShort(extraLen.toShort())
                if (e.zip64) { putLong(e.size); putLong(e.compressed) }
                if (bigOffset) putLong(e.offset)
            }
        })
    }

    private fun put(b: ByteBuffer) = sink.write(b.array(), 0, b.position())

    private fun dos(millis: Long): Pair<Int, Int> {
        val t = Instant.ofEpochMilli(millis).atZone(ZoneId.systemDefault()).toLocalDateTime()
        if (t.year < 1980) return 0 to ((1 shl 5) or 1)
        return ((t.hour shl 11) or (t.minute shl 5) or (t.second / 2)) to (((t.year - 1980) shl 9) or (t.monthValue shl 5) or t.dayOfMonth)
    }

    private companion object {
        const val BUFFER = 64 * 1024
        const val MAX_32 = 0xFFFFFFFFL
        const val MAX_16 = 0xFFFF
        // Under the 4 GB line with room for data that does not shrink, so a size is never wrong in either direction.
        const val ZIP64_ENTRY = 0xF0000000L
        const val VERSION_PLAIN = 20
        const val VERSION_ZIP64 = 45
        const val FLAGS = 0x0808 // data after the entry, names in UTF-8
        const val METHOD_DEFLATED = 8
    }
}
