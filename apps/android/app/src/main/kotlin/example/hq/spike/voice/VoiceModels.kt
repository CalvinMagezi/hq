package example.hq.spike.voice

import android.content.Context
import java.io.File
import java.io.IOException
import example.hq.spike.download.Downloads
import example.hq.spike.download.Item
import example.hq.spike.measure.DataPurpose
import org.apache.commons.compress.archivers.tar.TarArchiveInputStream
import org.apache.commons.compress.compressors.bzip2.BZip2CompressorInputStream

/**
 * One file the speech engines read. It comes from [url], which is either the file itself or a .tar.bz2 that holds it
 * at [entry]; [sha256] pins what was downloaded, so a changed upload is refused instead of run.
 */
data class ModelSource(val url: String, val sha256: String, val bytes: Long, val files: Map<String, String>)

/**
 * The speech models, fetched once from the sherpa-onnx releases on GitHub by the shared download engine (see [item]). Only the model travels: the
 * request carries no audio, no words and no account. Everything lives in app storage that is not backed up.
 */
class VoiceModels(private val context: Context) {
    private val root = File(context.noBackupFilesDir, "voice")
    val stt = File(root, "stt")

    /** Frees the 25 MB the retired spoken voice left behind; nothing reads it any more. */
    fun dropRetired() {
        File(root, RETIRED_TTS_DIR).deleteRecursively()
    }

    fun ready(): Boolean = SOURCES.all { (dir, source) -> doneMark(dir, source).isFile }

    private fun doneMark(dir: String, source: ModelSource) = File(File(root, dir), ".ok-" + source.sha256.take(MARK_CHARS))

    /** The engine's job for the one download: the archive is kept under [ARCHIVE] until [finish] has unpacked it. */
    fun item(): Item {
        val source = SOURCES.single().second
        return Item(ITEM_ID, NAME, source.url, source.bytes, source.sha256, DataPurpose.SpeechModel, UNPACKED_BYTES, ARCHIVE, speech = true)
    }

    /** Unpacks the checked archive, writes the done mark and deletes the archive. Safe to call twice: the second call finds it done. */
    fun finish(): Boolean = synchronized(LOCK) {
        if (ready()) return true
        val archive = File(Downloads.dirOf(context), ARCHIVE)
        if (!archive.isFile) return false
        val (dirName, source) = SOURCES.single()
        val dir = File(root, dirName).apply { mkdirs() }
        try {
            unpack(archive, dir, source.files)
        } catch (_: IOException) {
            dir.deleteRecursively()
            return false
        }
        doneMark(dirName, source).writeText(source.sha256)
        archive.delete()
        true
    }

    /** Removes the model from the phone; the next ask offers the download again. */
    fun delete() = synchronized(LOCK) {
        File(root, SOURCES.single().first).deleteRecursively()
        File(Downloads.dirOf(context), ARCHIVE).delete()
        Downloads.get(ITEM_ID)?.discard()
    }

    /** Bytes the model takes on this phone, from the files themselves. */
    fun onPhoneBytes(): Long = File(root, SOURCES.single().first).walkTopDown().filter { it.isFile && !it.name.startsWith(".ok-") }.sumOf { it.length() }

    /** Takes only the named entries, under the names this app chose, so an archive's own paths are never trusted. */
    private fun unpack(archive: File, dir: File, wanted: Map<String, String>) {
        val found = mutableSetOf<String>()
        TarArchiveInputStream(BZip2CompressorInputStream(archive.inputStream().buffered())).use { tar ->
            while (true) {
                val entry = tar.nextEntry ?: break
                val name = wanted[entry.name] ?: continue
                val tmp = File(dir, "$name.part")
                tmp.outputStream().use { tar.copyTo(it) }
                if (!tmp.renameTo(File(dir, name))) throw IOException("could not place $name")
                found += entry.name
            }
        }
        val missing = wanted.keys - found
        if (missing.isNotEmpty()) throw IOException("model archive is missing ${missing.joinToString()}")
    }

    companion object {
        private const val MARK_CHARS = 12
        private const val RELEASES = "https://github.com/k2-fsa/sherpa-onnx/releases/download"
        private const val STT_DIR = "sherpa-onnx-streaming-zipformer-en-kroko-2025-08-06"

        const val STT_ENCODER = "encoder.onnx"
        const val STT_DECODER = "decoder.onnx"
        const val STT_JOINER = "joiner.onnx"
        const val STT_TOKENS = "tokens.txt"

        /**
         * Streaming English recognition: Kroko (CC-BY-SA 4.0, trained on everyday speech; the LibriSpeech model
         * before it lost first words and technical terms with a real voice). Answers are shown, not spoken.
         */
        val SOURCES: List<Pair<String, ModelSource>> = listOf(
            "stt" to ModelSource(
                "$RELEASES/asr-models/$STT_DIR.tar.bz2",
                "c8676e5ff9ac2a85296e53ee0fd4d5fb1db6770e7a7647166eeafe349ade6834",
                57_267_600,
                mapOf(
                    "$STT_DIR/encoder.onnx" to STT_ENCODER,
                    "$STT_DIR/decoder.onnx" to STT_DECODER,
                    "$STT_DIR/joiner.onnx" to STT_JOINER,
                    "$STT_DIR/tokens.txt" to STT_TOKENS,
                ),
            ),
        )

        /** Where the spoken voice's models lived before answers became text only; removed from phones that have it. */
        const val RETIRED_TTS_DIR = "tts"

        const val ITEM_ID = "speech"

        /** What the download weighs, pinned with the checksum. */
        val SIZE_BYTES: Long get() = SOURCES.single().second.bytes

        private const val NAME = "Speech model"
        private const val ARCHIVE = "speech-model.tar.bz2"
        private val LOCK = Any()

        // Placeholder until measured on a phone: the four unpacked files of the Kroko model, rounded up. Device check in the report.
        const val UNPACKED_BYTES = 120_000_000L
    }
}
