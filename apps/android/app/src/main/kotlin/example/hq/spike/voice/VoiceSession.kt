package example.hq.spike.voice

import android.content.Context
import android.util.Log
import example.hq.spike.data.ChatConnection
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
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
class VoiceSession(app: Context, private val scope: CoroutineScope, private val connection: ChatConnection) {
    private val models = VoiceModels(app)
    private val ears = Ears(app)
    private val mouth = Mouth(app, scope)
    private val turn = VoiceTurn(connection.events) { content, thread, clientId -> connection.sendChat(content, thread, clientId) }
    private val _state = MutableStateFlow(VoiceState())
    val state: StateFlow<VoiceState> = _state

    private var threadId: String? = null
    private var job: Job? = null

    fun micPressed() = when (_state.value.phase) {
        VoicePhase.NeedsModels -> download()
        VoicePhase.Listening -> ears.finishNow()
        VoicePhase.Preparing, VoicePhase.Downloading -> Unit
        VoicePhase.Thinking, VoicePhase.Speaking -> { interrupt(); listen() }
        VoicePhase.Idle -> listen()
    }

    fun listen() {
        job?.cancel()
        job = scope.launch {
            try {
                hearThenAsk()
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

    fun close() {
        job?.cancel()
        ears.close()
        mouth.close()
    }

    /** Fetches the models only after the owner tapped "Get" under the notice of what is downloaded and what is not sent. */
    private fun download() {
        job?.cancel()
        job = scope.launch {
            try {
                _state.update { VoiceState(VoicePhase.Downloading, notice = DOWNLOAD_NOTICE) }
                models.fetch { part -> _state.update { it.copy(progress = part) } }
                hearThenAsk()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "model download failed", e)
                _state.update { VoiceState(VoicePhase.NeedsModels, notice = DOWNLOAD_NOTICE, problem = DOWNLOAD_FAILED) }
            }
        }
    }

    private suspend fun hearThenAsk() {
        if (!models.ready()) {
            _state.update { VoiceState(VoicePhase.NeedsModels, notice = DOWNLOAD_NOTICE) }
            return
        }
        _state.update { VoiceState(VoicePhase.Preparing, reply = it.reply) }
        ears.prepare()
        mouth.prepare()
        _state.update { it.copy(phase = VoicePhase.Listening, heard = "", problem = null) }
        val words = ears.hear { partial -> _state.update { it.copy(heard = partial) } }.trim()
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
            is TurnOutput.Failed -> _state.update { it.copy(problem = out.reason) }
            TurnOutput.Done -> Unit
        }
    }

    private fun interrupt() {
        job?.cancel()
        ears.stop()
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
