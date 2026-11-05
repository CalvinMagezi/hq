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
    suspend fun hear(stop: AtomicBoolean, wait: Boolean = true, onRecording: () -> Unit, onPartial: (String) -> Unit, onLoad: (Load) -> Unit = {}): String =
        withContext(Dispatchers.IO) {
            if (wait) micLock.lock() else if (!micLock.tryLock()) throw Busy()
            try {
                hearLocked(stop, onRecording, onPartial, onLoad) { isActive }
            } finally {
                micLock.unlock()
            }
        }

    /** The recogniser loads while the microphone already records: what was said meanwhile waits in an [AudioBacklog]. */
    enum class Load { Waiting, Slow, Ready }

    /** [onRecording] fires once the microphone really records; opening it takes up to a second on a budget phone. */
    private fun hearLocked(stop: AtomicBoolean, onRecording: () -> Unit, onPartial: (String) -> Unit, onLoad: (Load) -> Unit, active: () -> Boolean): String {
        val loader = if (recognizer == null) Thread({ runCatching { synchronized(prepareLock) { if (recognizer == null) recognizer = OnlineRecognizer(null, config(models.stt)) } } }, "ears-load").also { it.start() } else null
        val minBuffer = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val mic = AudioRecord(MediaRecorder.AudioSource.VOICE_RECOGNITION, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuffer, BUFFER_BYTES))
        var stream: com.k2fsa.sherpa.onnx.OnlineStream? = null
        return try {
            if (mic.state != AudioRecord.STATE_INITIALIZED) throw IOException("microphone did not open")
            mic.startRecording()
            // Another app (a call, a recorder) holds the microphone: Android starts the stream but never records.
            if (MicAsk.blocked(mic.recordingState == AudioRecord.RECORDSTATE_RECORDING, android.os.Build.VERSION.SDK_INT, silenced(mic))) throw Busy()
            val chunk = ShortArray(CHUNK)
            var n = mic.read(chunk, 0, chunk.size)
            if (n < 0) throw IOException("microphone did not start")
            onRecording()
            val started = SystemClock.elapsedRealtime()
            val backlog = AudioBacklog(BACKLOG_CHUNKS)
            var announced = false
            var slow = false
            var shown = ""
            while (active() && !stop.get() && SystemClock.elapsedRealtime() - started < LONGEST_REQUEST_MS) {
                val engine = recognizer
                if (engine == null) {
                    if (loader != null && !loader.isAlive) throw IllegalStateException("recogniser did not load")
                    backlog.add(chunk, n)
                    if (!announced) { announced = true; onLoad(Load.Waiting) }
                    if (!slow && SystemClock.elapsedRealtime() - started > SLOW_LOAD_MS) { slow = true; onLoad(Load.Slow) }
                } else {
                    val st = stream ?: engine.createStream().also { s ->
                        stream = s
                        backlog.drain().forEach { piece -> s.acceptWaveform(FloatArray(piece.size) { piece[it] / PCM_SCALE }, RATE) }
                        if (announced) onLoad(Load.Ready)
                    }
                    st.acceptWaveform(FloatArray(n) { chunk[it] / PCM_SCALE }, RATE)
                    while (engine.isReady(st)) engine.decode(st)
                    val text = engine.getResult(st).text
                    if (text != shown) {
                        shown = text
                        onPartial(readable(text))
                    }
                    if (engine.isEndpoint(st)) break
                }
                n = mic.read(chunk, 0, chunk.size)
                if (n < 0) throw IOException("microphone read failed ($n)")
            }
            chunk.fill(0)
            val engine = recognizer
            val st = stream
            if (engine == null || st == null) "" else {
                st.inputFinished()
                while (engine.isReady(st)) engine.decode(st)
                readable(engine.getResult(st).text)
            }
        } finally {
            runCatching { mic.stop() }
            mic.release()
            stream?.release()
        }
    }

    /** From Android 10 the system can silence a client that is recording while another app has priority. */
    private fun silenced(mic: AudioRecord): Boolean {
        if (android.os.Build.VERSION.SDK_INT < 29) return false
        return mic.activeRecordingConfiguration?.isClientSilenced == true
    }

    private companion object {
        const val RATE = 16_000
        const val CHUNK = RATE / 10
        const val PCM_SCALE = 32_768f
        /** Three seconds of 16-bit audio, so a slow decode never makes Android drop what was said meanwhile. */
        const val BUFFER_BYTES = RATE * 2 * 3
        const val LONGEST_REQUEST_MS = 30_000L
        /** Past this the label falls back to Getting ready and the oldest audio is dropped. */
        const val SLOW_LOAD_MS = 2_500L
        const val BACKLOG_CHUNKS = 30
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
                modelType = "zipformer2",
            ),
            endpointConfig = EndpointConfig(
                rule1 = EndpointRule(false, NOTHING_SAID_S, 0f),
                rule2 = EndpointRule(true, END_PAUSE_S, 0f),
                rule3 = EndpointRule(false, 0f, LONGEST_UTTERANCE_S),
            ),
            enableEndpoint = true,
            decodingMethod = "greedy_search",
        )

        /** Text from a model that writes all capitals is lowered; a model that writes proper case keeps its names. */
        fun readable(text: String): String {
            val trimmed = text.trim()
            val cased = if (trimmed.any { it.isLowerCase() }) trimmed else trimmed.lowercase()
            return cased.replaceFirstChar { it.titlecase() }
        }
    }
}

/** The last few seconds of audio heard before the recogniser was ready: the oldest pieces go first once it is full. */
class AudioBacklog(private val maxChunks: Int) {
    private val pieces = ArrayDeque<ShortArray>()

    fun add(chunk: ShortArray, n: Int) {
        pieces.addLast(chunk.copyOf(n))
        while (pieces.size > maxChunks) pieces.removeFirst()
    }

    /** Everything kept, oldest first, and empties the backlog. */
    fun drain(): List<ShortArray> = pieces.toList().also { pieces.clear() }
}
