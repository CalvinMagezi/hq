package example.hq.spike.voice

import android.content.Context
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.security.MessageDigest
import example.hq.spike.measure.Counters
import example.hq.spike.measure.DataPurpose
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import org.apache.commons.compress.archivers.tar.TarArchiveInputStream
import org.apache.commons.compress.compressors.bzip2.BZip2CompressorInputStream

/**
 * One file the speech engines read. It comes from [url], which is either the file itself or a .tar.bz2 that holds it
 * at [entry]; [sha256] pins what was downloaded, so a changed upload is refused instead of run.
 */
data class ModelSource(val url: String, val sha256: String, val bytes: Long, val files: Map<String, String>)

/**
 * The speech models, fetched once from the sherpa-onnx releases on GitHub. Only the models travel: the download
 * request carries no audio, no words and no account. Everything lives in app storage that is not backed up.
 */
class VoiceModels(private val context: Context, private val http: OkHttpClient = OkHttpClient()) {
    private val root = File(context.noBackupFilesDir, "voice")
    val stt = File(root, "stt")

    /** Frees the 25 MB the retired spoken voice left behind; nothing reads it any more. */
    fun dropRetired() {
        File(root, RETIRED_TTS_DIR).deleteRecursively()
    }

    fun ready(): Boolean = SOURCES.all { (dir, source) -> doneMark(dir, source).isFile }

    /** Downloads, checks and unpacks whatever is missing; [progress] gets 0 to 1 over the whole set. */
    suspend fun fetch(progress: (Float) -> Unit) = withContext(Dispatchers.IO) {
        val total = SOURCES.sumOf { it.second.bytes }.toFloat()
        var before = 0L
        for ((dirName, source) in SOURCES) {
            val dir = File(root, dirName).apply { mkdirs() }
            if (!doneMark(dirName, source).isFile) fetchOne(dir, source) { got -> progress((before + got) / total) }
            doneMark(dirName, source).writeText(source.sha256)
            before += source.bytes
        }
        progress(1f)
    }

    private fun doneMark(dir: String, source: ModelSource) = File(File(root, dir), ".ok-" + source.sha256.take(MARK_CHARS))

    /**
     * A download cut short (screen off, lost signal) is kept and continued with a Range request next time; the
     * checksum still covers every byte, so a resumed file is trusted no more than a fresh one.
     */
    private suspend fun fetchOne(dir: File, source: ModelSource, progress: (Long) -> Unit) {
        val partial = File(dir, ".download-" + source.sha256.take(MARK_CHARS))
        if (partial.length() > source.bytes) partial.delete()
        val digest = MessageDigest.getInstance("SHA-256")
        var have = partial.length()
        if (have > 0) partial.inputStream().use { copy(it, null, digest) {} }
        if (have < source.bytes) {
            val request = Request.Builder().url(source.url).apply { if (have > 0) header("Range", "bytes=$have-") }.build()
            http.newCall(request).execute().use { response ->
                if (!response.isSuccessful) throw IOException("model download answered ${response.code}")
                val resumed = have > 0 && response.code == HTTP_PARTIAL
                if (!resumed) {
                    digest.reset()
                    have = 0
                }
                val base = have
                var read = 0L
                try {
                    FileOutputStream(partial, resumed).use { out -> copy(response.body.byteStream(), out, digest) { got -> read = got; progress(base + got) } }
                } finally {
                    Counters.get(context).addBytes(DataPurpose.SpeechModel, Counters.metered(context), read)
                }
            }
        }
        val got = digest.digest().joinToString("") { "%02x".format(it) }
        if (got != source.sha256) {
            partial.delete()
            throw IOException("model download did not match its pinned checksum")
        }
        if (source.url.endsWith(TAR_BZ2)) unpack(partial, dir, source.files) else partial.copyTo(File(dir, source.files.values.single()), overwrite = true)
        partial.delete()
    }

    private suspend fun copy(input: InputStream, out: OutputStream?, digest: MessageDigest, progress: (Long) -> Unit) {
        val buffer = ByteArray(BUFFER)
        var total = 0L
        while (true) {
            kotlin.coroutines.coroutineContext.ensureActive()
            val n = input.read(buffer)
            if (n < 0) break
            out?.write(buffer, 0, n)
            digest.update(buffer, 0, n)
            total += n
            progress(total)
        }
    }

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
        private const val TAR_BZ2 = ".tar.bz2"
        private const val HTTP_PARTIAL = 206
        private const val BUFFER = 64 * 1024
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

        /** What the owner is told before the one download. */
        val DOWNLOAD_MB = SOURCES.sumOf { it.second.bytes } / (1024 * 1024)
    }
}
