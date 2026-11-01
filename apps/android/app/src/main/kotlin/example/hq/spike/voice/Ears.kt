package example.hq.spike.voice

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.SystemClock
import com.k2fsa.sherpa.onnx.EndpointConfig
import com.k2fsa.sherpa.onnx.EndpointRule
import com.k2fsa.sherpa.onnx.FeatureConfig
import com.k2fsa.sherpa.onnx.OnlineModelConfig
import com.k2fsa.sherpa.onnx.OnlineRecognizer
import com.k2fsa.sherpa.onnx.OnlineRecognizerConfig
import com.k2fsa.sherpa.onnx.OnlineTransducerModelConfig
import java.io.File
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext

/**
 * Speech to text on the phone. Audio goes from the microphone into the recogniser in 100 ms pieces and is dropped
 * as soon as it is read: nothing is written to storage and nothing is sent anywhere.
 */
class Ears(app: Context) {
    private val models = VoiceModels(app)
    private var recognizer: OnlineRecognizer? = null
    @Volatile private var finishing = false
    private val lock = Any()

    suspend fun prepare() = withContext(Dispatchers.Default) {
        synchronized(lock) { if (recognizer == null) recognizer = OnlineRecognizer(null, config(models.stt)) }
    }

    /** Listens until a pause ends the request, [finishNow] is called, or the time limit; returns what was said. */
    // The voice screen asks for the microphone before it calls this.
    @SuppressLint("MissingPermission")
    suspend fun hear(onPartial: (String) -> Unit): String = withContext(Dispatchers.IO) {
        // Held for the whole loop so [close] cannot free the recogniser while it decodes; one read is 100 ms.
        synchronized(lock) { hearLocked(onPartial) { isActive } }
    }

    private fun hearLocked(onPartial: (String) -> Unit, active: () -> Boolean): String {
        val engine = recognizer ?: throw IllegalStateException("prepare() first")
        val stream = engine.createStream()
        val minBuffer = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val mic = AudioRecord(MediaRecorder.AudioSource.VOICE_RECOGNITION, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuffer, CHUNK * 2) * 2)
        finishing = false
        return try {
            if (mic.state != AudioRecord.STATE_INITIALIZED) throw IOException("microphone did not open")
            mic.startRecording()
            val chunk = ShortArray(CHUNK)
            val started = SystemClock.elapsedRealtime()
            var shown = ""
            while (active() && !finishing && SystemClock.elapsedRealtime() - started < LONGEST_REQUEST_MS) {
                val n = mic.read(chunk, 0, chunk.size)
                if (n < 0) throw IOException("microphone read failed ($n)")
                stream.acceptWaveform(FloatArray(n) { chunk[it] / PCM_SCALE }, RATE)
                while (engine.isReady(stream)) engine.decode(stream)
                val text = engine.getResult(stream).text
                if (text != shown) {
                    shown = text
                    onPartial(readable(text))
                }
                if (engine.isEndpoint(stream)) break
            }
            chunk.fill(0)
            stream.inputFinished()
            while (engine.isReady(stream)) engine.decode(stream)
            readable(engine.getResult(stream).text)
        } finally {
            runCatching { mic.stop() }
            mic.release()
            stream.release()
        }
    }

    fun finishNow() {
        finishing = true
    }

    fun stop() = finishNow()

    fun close() {
        finishing = true
        synchronized(lock) {
            recognizer?.release()
            recognizer = null
        }
    }

    private companion object {
        const val RATE = 16_000
        const val CHUNK = RATE / 10
        const val PCM_SCALE = 32_768f
        const val LONGEST_REQUEST_MS = 30_000L
        const val FEATURE_DIM = 80
        const val THREADS = 2
        /** Silence that ends a request once something was said; short enough to feel like a conversation. */
        const val END_PAUSE_S = 1.0f
        /** Silence that gives up when nothing was said at all. */
        const val NOTHING_SAID_S = 4.0f
        const val LONGEST_UTTERANCE_S = 30f

        fun config(dir: File) = OnlineRecognizerConfig(
            featConfig = FeatureConfig(sampleRate = RATE, featureDim = FEATURE_DIM),
            modelConfig = OnlineModelConfig(
                transducer = OnlineTransducerModelConfig(
                    encoder = File(dir, VoiceModels.STT_ENCODER).path,
                    decoder = File(dir, VoiceModels.STT_DECODER).path,
                    joiner = File(dir, VoiceModels.STT_JOINER).path,
                ),
                tokens = File(dir, VoiceModels.STT_TOKENS).path,
                numThreads = THREADS,
                modelType = "zipformer",
            ),
            endpointConfig = EndpointConfig(
                rule1 = EndpointRule(false, NOTHING_SAID_S, 0f),
                rule2 = EndpointRule(true, END_PAUSE_S, 0f),
                rule3 = EndpointRule(false, 0f, LONGEST_UTTERANCE_S),
            ),
            enableEndpoint = true,
            decodingMethod = "greedy_search",
        )

        /** The model writes capitals with no punctuation; this reads like a typed question. */
        fun readable(text: String): String {
            val lower = text.trim().lowercase()
            return lower.replaceFirstChar { it.titlecase() }
        }
    }
}
