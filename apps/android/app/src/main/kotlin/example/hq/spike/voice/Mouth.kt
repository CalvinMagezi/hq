package example.hq.spike.voice

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.util.Log
import com.k2fsa.sherpa.onnx.OfflineTts
import com.k2fsa.sherpa.onnx.OfflineTtsConfig
import com.k2fsa.sherpa.onnx.OfflineTtsModelConfig
import com.k2fsa.sherpa.onnx.OfflineTtsVitsModelConfig
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Text to speech on the phone: sentences queue up and are spoken in order while later ones are still arriving.
 * [stop] silences it at once and drops what was queued. Other audio is ducked while it speaks.
 */
class Mouth(app: Context, scope: CoroutineScope) {
    private val models = VoiceModels(app)
    private val audio = app.getSystemService(AudioManager::class.java)
    private val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ASSISTANT).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build()
    private val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_MAY_DUCK).setAudioAttributes(attributes).build()
    private val engineLock = Any()
    @Volatile private var engine: OfflineTts? = null
    @Volatile private var track: AudioTrack? = null
    private val queue = Channel<Queued>(Channel.UNLIMITED)
    private val pending = MutableStateFlow(0)

    /** Guards [generation] and [framesWritten] together, so a write that finishes after [stop] cannot count. */
    private val frameLock = Any()
    /** Bumped by [stop]; speech started under an older number ends at its next piece. */
    private var generation = 0
    private var framesWritten = 0L

    private class Queued(val text: String, val generation: Int)

    init {
        // The engine and the track are used and released only here, never while a sentence is generating.
        scope.launch(Dispatchers.Default) {
            try {
                for (item in queue) speakSafely(item)
            } finally {
                releaseTrack()
                synchronized(engineLock) {
                    engine?.release()
                    engine = null
                }
            }
        }
    }

    /** One bad sentence (a busy audio device, a refused track) is logged and skipped; the worker keeps going. */
    private fun speakSafely(item: Queued) {
        try {
            if (item.generation == currentGeneration()) speak(item)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Log.w(TAG, "a sentence could not be spoken", e)
            releaseTrack()
        } finally {
            done()
        }
    }

    private fun done() = pending.update { maxOf(0, it - 1) }

    private fun currentGeneration() = synchronized(frameLock) { generation }

    suspend fun prepare() = withContext(Dispatchers.Default) {
        synchronized(engineLock) { if (engine == null) engine = OfflineTts(null, config(models.tts)) }
    }

    fun say(sentence: String) {
        val text = SpeechWords.forVoice(sentence)
        if (text.isBlank()) return
        pending.update { it + 1 }
        if (queue.trySend(Queued(text, currentGeneration())).isFailure) done()
    }

    /** Waits until everything queued has been spoken and has left the speaker, then gives the audio back. */
    suspend fun drain() {
        if (withTimeoutOrNull(SPEAKING_LIMIT_MS) { pending.first { it == 0 } } == null) return stop()
        track?.let { t ->
            withTimeoutOrNull(PLAYOUT_LIMIT_MS) {
                while (t.playState == AudioTrack.PLAYSTATE_PLAYING && t.playbackHeadPosition.toLong() < synchronized(frameLock) { framesWritten }) {
                    delay(PLAYOUT_POLL_MS)
                }
            }
            runCatching { t.pause() }
        }
        audio.abandonAudioFocusRequest(focus)
    }

    fun stop() {
        synchronized(frameLock) {
            generation++
            framesWritten = 0
        }
        while (queue.tryReceive().isSuccess) done()
        track?.let {
            // pause() silences at once (stop() would play out the buffer) and wakes a write blocked on a full buffer.
            runCatching { it.pause() }
            runCatching { it.flush() }
        }
        audio.abandonAudioFocusRequest(focus)
    }

    private fun speak(item: Queued) {
        val tts = synchronized(engineLock) { engine } ?: return
        val out = track ?: newTrack(tts.sampleRate()).also { track = it }
        audio.requestAudioFocus(focus)
        if (out.playState != AudioTrack.PLAYSTATE_PLAYING) out.play()
        tts.generateWithCallback(item.text, SPEAKER, SPEED, Player(out, item.generation))
    }

    private fun releaseTrack() {
        track?.let { runCatching { it.release() } }
        track = null
    }

    /**
     * A class, not a lambda: the native side calls `invoke([F)Ljava/lang/Integer;`, which an invokedynamic lambda
     * (Kotlin 2's default) does not have, and the process aborts on the first piece of audio.
     */
    private inner class Player(private val out: AudioTrack, private val ofGeneration: Int) : (FloatArray) -> Int {
        override fun invoke(samples: FloatArray): Int {
            if (ofGeneration != currentGeneration()) return STOP
            val wrote = out.write(samples, 0, samples.size, AudioTrack.WRITE_BLOCKING)
            val current = synchronized(frameLock) {
                val same = ofGeneration == generation
                if (same && wrote > 0) framesWritten += wrote
                same
            }
            return if (wrote < 0 || !current) STOP else GO_ON
        }
    }

    private fun newTrack(rate: Int): AudioTrack {
        val format = AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_FLOAT).setSampleRate(rate).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build()
        val minimum = AudioTrack.getMinBufferSize(rate, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_FLOAT)
        if (minimum <= 0) throw IllegalStateException("audio output refused this format ($minimum)")
        return AudioTrack.Builder().setAudioAttributes(attributes).setAudioFormat(format).setBufferSizeInBytes(minimum * BUFFER_FACTOR)
            .setTransferMode(AudioTrack.MODE_STREAM).build()
    }

    private companion object {
        const val TAG = "HqVoice"
        const val SPEAKER = 0
        const val SPEED = 1.0f
        const val THREADS = 2
        const val STOP = 0
        const val GO_ON = 1
        const val BUFFER_FACTOR = 4
        /** A reply longer than this is cut off rather than holding the sheet in Speaking for ever. */
        const val SPEAKING_LIMIT_MS = 180_000L
        const val PLAYOUT_LIMIT_MS = 15_000L
        const val PLAYOUT_POLL_MS = 50L

        fun config(dir: File) = OfflineTtsConfig(
            model = OfflineTtsModelConfig(
                vits = OfflineTtsVitsModelConfig(
                    model = File(dir, VoiceModels.TTS_MODEL).path,
                    lexicon = File(dir, VoiceModels.TTS_LEXICON).path,
                    tokens = File(dir, VoiceModels.TTS_TOKENS).path,
                ),
                numThreads = THREADS,
            ),
        )
    }
}
