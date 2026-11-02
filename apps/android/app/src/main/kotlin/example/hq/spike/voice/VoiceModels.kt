package example.hq.spike.voice

import android.content.Context
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.security.MessageDigest
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
class VoiceModels(context: Context, private val http: OkHttpClient = OkHttpClient()) {
    private val root = File(context.noBackupFilesDir, "voice")
    val stt = File(root, "stt")
    val tts = File(root, "tts")

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
                FileOutputStream(partial, resumed).use { out -> copy(response.body.byteStream(), out, digest) { got -> progress(base + got) } }
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
        private const val TTS_DIR = "vits-piper-en_US-ljspeech-medium-int8"

        const val STT_ENCODER = "encoder.onnx"
        const val STT_DECODER = "decoder.onnx"
        const val STT_JOINER = "joiner.onnx"
        const val STT_TOKENS = "tokens.txt"
        const val TTS_MODEL = "voice.onnx"
        const val TTS_TOKENS = "tokens.txt"
        const val TTS_LEXICON = "lexicon-en-us.txt"

        /**
         * Streaming English recognition: Kroko (CC-BY-SA 4.0, trained on everyday speech; the LibriSpeech model
         * before it lost first words and technical terms with a real voice). The LJ Speech voice is from
         * public-domain recordings.
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
            "tts" to ModelSource(
                "$RELEASES/tts-models/$TTS_DIR.tar.bz2",
                "24dc3bd77dd48c291e52c297878d3437c9492f245d823d7f6a06c4bbb67f4b6b",
                21_090_429,
                mapOf("$TTS_DIR/en_US-ljspeech-medium.onnx" to TTS_MODEL, "$TTS_DIR/tokens.txt" to TTS_TOKENS),
            ),
            "tts" to ModelSource(
                "$RELEASES/tts-models/lexicon-en-us.txt",
                "8b67ae549a465eee78f92ecb74644ad90c3c58ce65f19c68baf85878a10ef73a",
                6_305_773,
                mapOf(TTS_LEXICON to TTS_LEXICON),
            ),
        )

        /** What the owner is told before the one download. */
        val DOWNLOAD_MB = SOURCES.sumOf { it.second.bytes } / (1024 * 1024)
    }
}
