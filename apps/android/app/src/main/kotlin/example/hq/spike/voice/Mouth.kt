package example.hq.spike.voice

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import com.k2fsa.sherpa.onnx.OfflineTts
import com.k2fsa.sherpa.onnx.OfflineTtsConfig
import com.k2fsa.sherpa.onnx.OfflineTtsModelConfig
import com.k2fsa.sherpa.onnx.OfflineTtsVitsModelConfig
import java.io.File
import java.util.concurrent.atomic.AtomicInteger
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
 * [stop] silences it at once and drops what was queued.
 */
class Mouth(app: Context, scope: CoroutineScope) {
    private val models = VoiceModels(app)
    @Volatile private var engine: OfflineTts? = null
    @Volatile private var track: AudioTrack? = null
    private val queue = Channel<Queued>(Channel.UNLIMITED)
    private val pending = MutableStateFlow(0)
    /** Bumped by [stop]; speech started under an older number ends at its next piece. */
    private val generation = AtomicInteger()
    @Volatile private var framesWritten = 0L

    private class Queued(val text: String, val generation: Int)

    init {
        // The engine and the track are released here, on the thread that uses them, never while a sentence is generating.
        scope.launch(Dispatchers.Default) {
            try {
                for (item in queue) {
                    try {
                        if (item.generation == generation.get()) speak(item)
                    } finally {
                        done()
                    }
                }
            } finally {
                track?.release()
                track = null
                engine?.release()
                engine = null
            }
        }
    }

    private fun done() = pending.update { maxOf(0, it - 1) }

    suspend fun prepare() = withContext(Dispatchers.Default) {
        if (engine == null) engine = OfflineTts(null, config(models.tts))
    }

    fun say(sentence: String) {
        val text = SpeechWords.forVoice(sentence)
        if (text.isBlank()) return
        pending.update { it + 1 }
        if (queue.trySend(Queued(text, generation.get())).isFailure) done()
    }

    /** Waits until everything queued has been spoken and has left the speaker. */
    suspend fun drain() {
        pending.first { it == 0 }
        val t = track ?: return
        withTimeoutOrNull(PLAYOUT_LIMIT_MS) {
            while (t.playState == AudioTrack.PLAYSTATE_PLAYING && t.playbackHeadPosition.toLong() < framesWritten) delay(PLAYOUT_POLL_MS)
        }
    }

    fun stop() {
        generation.incrementAndGet()
        while (queue.tryReceive().isSuccess) done()
        track?.let {
            // stop() wakes a write that is blocked on a full buffer, so the speaking thread notices the new generation.
            runCatching { it.stop() }
            runCatching { it.flush() }
        }
        framesWritten = 0
    }

    fun close() {
        stop()
        queue.close()
    }

    private fun speak(item: Queued) {
        val tts = engine ?: return
        val out = track ?: newTrack(tts.sampleRate()).also { track = it }
        if (out.playState != AudioTrack.PLAYSTATE_PLAYING) out.play()
        tts.generateWithCallback(item.text, SPEAKER, SPEED) { samples ->
            if (item.generation != generation.get()) return@generateWithCallback STOP
            val wrote = out.write(samples, 0, samples.size, AudioTrack.WRITE_BLOCKING)
            if (wrote > 0) framesWritten += wrote
            if (wrote < 0 || item.generation != generation.get()) STOP else GO_ON
        }
    }

    private fun newTrack(rate: Int): AudioTrack {
        val format = AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_FLOAT).setSampleRate(rate).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build()
        val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ASSISTANT).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build()
        val size = AudioTrack.getMinBufferSize(rate, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_FLOAT)
        return AudioTrack.Builder().setAudioAttributes(attributes).setAudioFormat(format).setBufferSizeInBytes(size * BUFFER_FACTOR)
            .setTransferMode(AudioTrack.MODE_STREAM).build()
    }

    private companion object {
        const val SPEAKER = 0
        const val SPEED = 1.0f
        const val THREADS = 2
        const val STOP = 0
        const val GO_ON = 1
        const val BUFFER_FACTOR = 4
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
