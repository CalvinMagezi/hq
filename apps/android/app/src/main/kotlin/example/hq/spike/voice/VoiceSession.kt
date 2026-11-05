package example.hq.spike.voice

import android.content.Context
import android.os.SystemClock
import android.util.Log
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import example.hq.spike.R
import example.hq.spike.data.ChatConnection
import example.hq.spike.download.DlState
import example.hq.spike.download.Downloads
import example.hq.spike.download.LargeDownload
import example.hq.spike.ui.shell.Words
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class VoicePhase { NeedsModel, Downloading, ModelReady, Preparing, Listening, Thinking, LookingUp, Answering, Idle }

data class VoiceState(
    val phase: VoicePhase = VoicePhase.Idle,
    val heard: String = "",
    val reply: String = "",
    val problem: String? = null,
)

/**
 * One open voice sheet: hear, ask, show the answer as it arrives, then wait for the next tap. A tap while HQ is
 * answering interrupts it and listens again. Follow-up questions stay in the same chat thread.
 */
class VoiceSession(private val app: Context, private val scope: CoroutineScope, private val connection: ChatConnection) {
    private val models = VoiceModels(app)
    private val words = Words.of(app)
    private val ears = VoiceEngines.ears(app)
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
        scope.launch(Dispatchers.IO) { models.dropRetired() }
        scope.launch { Downloads.states.map { it[VoiceModels.ITEM_ID] }.distinctUntilChanged().collect(::onDownload) }
    }

    /** The model's job on the shared download engine, made on first use so the sheet's row has something to act on. */
    fun engine(): LargeDownload = Downloads.engine(app, models.item())

    private fun waitingPhase(s: DlState?) = when (s) {
        DlState.WaitingWifi, is DlState.Downloading, is DlState.PausedWifi, is DlState.PausedOwner, is DlState.CutOff, DlState.Checking -> VoicePhase.Downloading
        else -> VoicePhase.NeedsModel
    }

    /** Follows the download while the sheet shows the model's row. When the model lands it says so and waits: nothing listens by itself. */
    private fun onDownload(s: DlState?) {
        val phase = _state.value.phase
        if (phase != VoicePhase.NeedsModel && phase != VoicePhase.Downloading) return
        if (s is DlState.Done) {
            scope.launch(Dispatchers.IO) {
                if (models.finish()) _state.update { VoiceState(VoicePhase.ModelReady) }
                else _state.update { it.copy(phase = VoicePhase.NeedsModel, problem = words.s(R.string.speech_unpack_failed)) }
            }
        } else {
            _state.update { it.copy(phase = waitingPhase(s)) }
        }
    }

    fun micPressed() = when (_state.value.phase) {
        VoicePhase.NeedsModel, VoicePhase.Downloading, VoicePhase.Preparing -> Unit
        VoicePhase.Listening -> hearing.set(true)
        VoicePhase.ModelReady -> listen()
        VoicePhase.Thinking, VoicePhase.LookingUp, VoicePhase.Answering -> { interrupt(); listen() }
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
                refuse(words.s(R.string.voice_engine_failed))
            }
        }
    }

    fun refuse(problem: String) = _state.update { it.copy(phase = VoicePhase.Idle, problem = problem) }

    /** The sheet left the screen: stop hearing, and stop HQ's answer if one is running. */
    fun quiet() {
        if (_state.value.phase != VoicePhase.Idle) interrupt()
        _state.update { it.copy(phase = VoicePhase.Idle) }
    }

    /** The recogniser stays loaded for the next request (see [VoiceEngines]); only this sheet's work stops. */
    fun close() {
        job?.cancel()
        hearing.set(true)
    }

    private suspend fun hearThenAsk() {
        if (!models.ready()) {
            _state.update { VoiceState(waitingPhase(engine().state.value)) }
            return
        }
        _state.update { VoiceState(VoicePhase.Preparing, reply = it.reply) }
        ears.prepare()
        _state.update { it.copy(heard = "", problem = null) }
        val stop = AtomicBoolean(false).also { hearing = it }
        val said = ears.hear(
            stop,
            onRecording = { _state.update { it.copy(phase = VoicePhase.Listening) } },
            onPartial = { partial -> _state.update { it.copy(heard = partial) } },
        ).trim()
        if (said.isEmpty()) return refuse(words.s(R.string.voice_nothing_heard))
        val question = Invocation.strip(said)
        _state.update { it.copy(phase = VoicePhase.Thinking, heard = question, reply = "") }
        turn.ask(question, threadId).collect(::onOutput)
        _state.update { it.copy(phase = VoicePhase.Idle) }
    }

    private fun onOutput(out: TurnOutput) {
        when (out) {
            is TurnOutput.Thread -> threadId = out.id
            is TurnOutput.Text -> _state.update { it.copy(phase = VoicePhase.Answering, reply = it.reply + out.chunk) }
            is TurnOutput.Failed -> _state.update { it.copy(problem = out.reason) }
            TurnOutput.Working -> _state.update { it.copy(phase = VoicePhase.LookingUp) }
            TurnOutput.Done -> Unit
        }
    }

    private fun interrupt() {
        job?.cancel()
        hearing.set(true)
        val phase = _state.value.phase
        if (phase == VoicePhase.Thinking || phase == VoicePhase.LookingUp || phase == VoicePhase.Answering) threadId?.let(connection::stop)
    }

    private companion object {
        const val TAG = "HqVoice"
    }
}
