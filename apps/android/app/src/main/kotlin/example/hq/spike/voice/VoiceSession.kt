package example.hq.spike.voice

import android.content.Context
import android.os.SystemClock
import android.util.Log
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import example.hq.spike.data.ChatConnection
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class VoicePhase(val label: String, val action: String) {
    NeedsModels("Voice needs its speech models", "Get"),
    Downloading("Downloading speech models", "Wait"),
    Preparing("Getting ready", "Wait"),
    Listening("Listening", "Done"),
    Thinking("Thinking", "Stop"),
    Speaking("Speaking", "Stop"),
    Idle("Tap to ask", "Ask"),
}

data class VoiceState(
    val phase: VoicePhase = VoicePhase.Idle,
    val heard: String = "",
    val reply: String = "",
    val problem: String? = null,
    val notice: String? = null,
    val progress: Float? = null,
)

/**
 * One open voice sheet: hear, ask, speak, then wait for the next tap. A tap while HQ thinks or speaks interrupts
 * it and listens again. Follow-up questions stay in the same chat thread.
 */
class VoiceSession(private val app: Context, private val scope: CoroutineScope, private val connection: ChatConnection) {
    private val models = VoiceModels(app)
    private val ears = VoiceEngines.ears(app)
    private val mouth = VoiceEngines.mouth(app)
    private val turn = VoiceTurn(connection.events, connection.connected, { content, thread, clientId -> connection.sendChat(content, thread, clientId) })
    private val _state = MutableStateFlow(VoiceState())
    val state: StateFlow<VoiceState> = _state

    private var threadId: String? = null
    private var job: Job? = null
    /** This sheet's own stop switch for the request it is hearing; dictation for other apps has its own. */
    @Volatile private var hearing = AtomicBoolean(true)

    init {
        // Phase changes with a monotonic clock, for latency runs from adb. Never the words: logcat is not private enough.
        scope.launch {
            _state.map { it.phase }.distinctUntilChanged().collect { Log.i(TAG, "phase=$it at=${SystemClock.elapsedRealtime()}") }
        }
    }

    fun micPressed() = when (_state.value.phase) {
        VoicePhase.NeedsModels -> download()
        VoicePhase.Listening -> hearing.set(true)
        VoicePhase.Preparing, VoicePhase.Downloading -> Unit
        VoicePhase.Thinking, VoicePhase.Speaking -> { interrupt(); listen() }
        VoicePhase.Idle -> listen()
    }

    fun listen() = run { hearThenAsk() }

    /** Every piece of work the sheet starts goes through here: one at a time, and a failure is shown, never a crash. */
    private fun run(work: suspend () -> Unit) {
        job?.cancel()
        job = scope.launch {
            try {
                work()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "voice turn failed", e)
                refuse(ENGINE_FAILED)
            }
        }
    }

    fun refuse(problem: String) = _state.update { it.copy(phase = VoicePhase.Idle, problem = problem) }

    /** The sheet left the screen: stop hearing and speaking, and stop HQ's answer if one is running. */
    fun quiet() {
        if (_state.value.phase != VoicePhase.Idle) interrupt()
        _state.update { it.copy(phase = VoicePhase.Idle) }
    }

    /** The engines stay loaded for the next request (see [VoiceEngines]); only this sheet's work stops. */
    fun close() {
        job?.cancel()
        hearing.set(true)
        mouth.stop()
    }

    /** Starts the models' download only after the owner tapped "Get" under the notice of what is fetched and what is not sent. */
    private fun download() = run { downloadThenListen(VoiceEngines.download(app)) }

    /** Follows the process-wide download; leaving the sheet stops this wait, not the download. */
    private suspend fun downloadThenListen(running: Deferred<Unit>) {
        _state.update { VoiceState(VoicePhase.Downloading, notice = DOWNLOAD_NOTICE) }
        val showing = scope.launch { VoiceEngines.progress.collect { part -> _state.update { it.copy(progress = part) } } }
        try {
            running.await()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            Log.w(TAG, "model download failed", e)
            _state.update { VoiceState(VoicePhase.NeedsModels, notice = DOWNLOAD_NOTICE, problem = DOWNLOAD_FAILED) }
            return
        } finally {
            showing.cancel()
        }
        hearThenAsk()
    }

    private suspend fun hearThenAsk() {
        if (!models.ready()) {
            val running = VoiceEngines.downloading()
            if (running != null) return downloadThenListen(running)
            _state.update { VoiceState(VoicePhase.NeedsModels, notice = DOWNLOAD_NOTICE) }
            return
        }
        _state.update { VoiceState(VoicePhase.Preparing, reply = it.reply) }
        // The voice loads while the owner speaks; it is only needed once the request has been heard.
        val voiceReady = scope.async { mouth.prepare() }
        ears.prepare()
        _state.update { it.copy(heard = "", problem = null) }
        val stop = AtomicBoolean(false).also { hearing = it }
        val words = ears.hear(
            stop,
            onRecording = { _state.update { it.copy(phase = VoicePhase.Listening) } },
            onPartial = { partial -> _state.update { it.copy(heard = partial) } },
        ).trim()
        voiceReady.await()
        if (words.isEmpty()) return refuse(NOTHING_HEARD)
        _state.update { it.copy(phase = VoicePhase.Thinking, heard = words, reply = "") }
        turn.ask(words, threadId).collect(::onOutput)
        mouth.drain()
        _state.update { it.copy(phase = VoicePhase.Idle) }
    }

    private fun onOutput(out: TurnOutput) {
        when (out) {
            is TurnOutput.Thread -> threadId = out.id
            is TurnOutput.Say -> {
                _state.update { it.copy(phase = VoicePhase.Speaking, reply = (it.reply + " " + out.sentence).trim()) }
                mouth.say(out.sentence)
            }
            is TurnOutput.Failed -> {
                // Said as well as shown: someone who asked by voice may not be looking at the screen.
                _state.update { it.copy(phase = VoicePhase.Speaking, problem = out.reason) }
                mouth.say(out.reason)
            }
            TurnOutput.Done -> Unit
        }
    }

    private fun interrupt() {
        job?.cancel()
        hearing.set(true)
        mouth.stop()
        if (_state.value.phase == VoicePhase.Thinking || _state.value.phase == VoicePhase.Speaking) threadId?.let(connection::stop)
    }

    private companion object {
        const val TAG = "HqVoice"
        const val NOTHING_HEARD = "I did not catch that. Tap and try again."
        const val ENGINE_FAILED = "Voice could not start on this phone. The details are in the app log."
        const val DOWNLOAD_FAILED = "The download did not finish or did not match. Check the connection and tap Get again."
        val DOWNLOAD_NOTICE = "Hearing and speaking happen on this phone, so it needs about ${VoiceModels.DOWNLOAD_MB} MB of speech " +
            "models, downloaded once from the sherpa-onnx project on GitHub. The download sends nothing about you. " +
            "After that, your voice never leaves the phone: only the words of your request go to HQ's chat model."
    }
}
