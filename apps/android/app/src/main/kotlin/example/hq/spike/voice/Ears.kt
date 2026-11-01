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
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.locks.ReentrantLock
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext

/**
 * Speech to text on the phone. Audio goes from the microphone into the recogniser in 100 ms pieces and is dropped
 * as soon as it is read: nothing is written to storage and nothing is sent anywhere. The words go back only to the
 * caller: the voice sheet, or an app that asked for dictation through [OnDeviceRecognitionService].
 */
class Ears(app: Context) {
    private val models = VoiceModels(app)
    @Volatile private var recognizer: OnlineRecognizer? = null
    private val prepareLock = Any()
    /** One microphone user at a time: the voice sheet, or another app's dictation. */
    private val micLock = ReentrantLock()

    /** Another listener holds the microphone. */
    class Busy : Exception("the microphone is in use by another request")

    suspend fun prepare() = withContext(Dispatchers.Default) {
        synchronized(prepareLock) { if (recognizer == null) recognizer = OnlineRecognizer(null, config(models.stt)) }
    }

    /**
     * Listens until a pause ends the request, [stop] is set, or the time limit; returns what was said. Each caller
     * owns its [stop] flag, so ending one request never ends another. With [wait] false a busy microphone throws
     * [Busy] at once instead of queueing.
     */
    // Every caller holds the microphone permission: the voice screen asks for it, and Android checks dictation callers.
    @SuppressLint("MissingPermission")
    suspend fun hear(stop: AtomicBoolean, wait: Boolean = true, onRecording: () -> Unit, onPartial: (String) -> Unit): String =
        withContext(Dispatchers.IO) {
            if (wait) micLock.lock() else if (!micLock.tryLock()) throw Busy()
            try {
                hearLocked(stop, onRecording, onPartial) { isActive }
            } finally {
                micLock.unlock()
            }
        }

    /** [onRecording] fires once the microphone really records; opening it takes up to a second on a budget phone. */
    private fun hearLocked(stop: AtomicBoolean, onRecording: () -> Unit, onPartial: (String) -> Unit, active: () -> Boolean): String {
        val engine = recognizer ?: throw IllegalStateException("prepare() first")
        val minBuffer = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val mic = AudioRecord(MediaRecorder.AudioSource.VOICE_RECOGNITION, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuffer, BUFFER_BYTES))
        val stream = try {
            engine.createStream()
        } catch (e: RuntimeException) {
            mic.release()
            throw e
        }
        return try {
            if (mic.state != AudioRecord.STATE_INITIALIZED) throw IOException("microphone did not open")
            mic.startRecording()
            val chunk = ShortArray(CHUNK)
            var n = mic.read(chunk, 0, chunk.size)
            if (n < 0) throw IOException("microphone did not start")
            onRecording()
            val started = SystemClock.elapsedRealtime()
            var shown = ""
            while (active() && !stop.get() && SystemClock.elapsedRealtime() - started < LONGEST_REQUEST_MS) {
                stream.acceptWaveform(FloatArray(n) { chunk[it] / PCM_SCALE }, RATE)
                while (engine.isReady(stream)) engine.decode(stream)
                val text = engine.getResult(stream).text
                if (text != shown) {
                    shown = text
                    onPartial(readable(text))
                }
                if (engine.isEndpoint(stream)) break
                n = mic.read(chunk, 0, chunk.size)
                if (n < 0) throw IOException("microphone read failed ($n)")
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

    private companion object {
        const val RATE = 16_000
        const val CHUNK = RATE / 10
        const val PCM_SCALE = 32_768f
        /** Three seconds of 16-bit audio, so a slow decode never makes Android drop what was said meanwhile. */
        const val BUFFER_BYTES = RATE * 2 * 3
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
