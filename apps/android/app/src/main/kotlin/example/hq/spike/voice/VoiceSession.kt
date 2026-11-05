package example.hq.spike.voice

import android.content.Context
import android.os.SystemClock
import android.util.Log
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import example.hq.spike.R
import example.hq.spike.data.ChatConnection
import example.hq.spike.data.StoredMessage
import example.hq.spike.data.HqServices
import example.hq.spike.data.Connection
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull
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

enum class VoicePhase { NeedsModel, Downloading, ModelReady, Preparing, Listening, Thinking, LookingUp, Answering, Idle, MicNeeded, MicBlocked, Stopped, WaitingForHq }

data class VoiceState(
    val phase: VoicePhase = VoicePhase.Idle,
    val heard: String = "",
    val reply: String = "",
    val problem: String? = null,
    /** The silent tries in a row, so a second one can say more than the first. */
    val silent: Int = 0,
    /** The chat this question landed in, so the sheet can open it. */
    val threadId: String? = null,
    /** The turn ended waiting for the owner's OK; the sheet says so instead of showing an error. */
    val needsOk: Boolean = false,
    /** The recogniser is still loading while the microphone listens: [Cold] until it shows words. */
    val cold: Boolean = false,
    /** Loading took longer than 2.5 s, so the label falls back to Getting ready. */
    val slowLoad: Boolean = false,
    /** The socket to HQ dropped while HQ itself is still working. */
    val reconnecting: Boolean = false,
    /** An answer came after the sheet closed and no notice was posted. */
    val afterClose: Boolean = false,
)

/**
 * One open voice sheet: hear, ask, show the answer as it arrives, then wait for the next tap. A tap while HQ is
 * answering interrupts it and listens again. Follow-up questions stay in the same chat thread.
 */
class VoiceSession(
    private val app: Context,
    private val scope: CoroutineScope,
    private val connection: ChatConnection,
    private val status: StateFlow<Connection> = HqServices.get(app).status,
    private val handoff: VoiceHandoff = VoiceHandoff.get(app),
) {
    private val models = VoiceModels(app)
    private val words = Words.of(app)
    private val ears = VoiceEngines.ears(app)
    private val client = HqServices.get(app).client
    private val turn = VoiceTurn(
        connection.events, connection.connected, { content, thread, clientId -> connection.sendChat(content, thread, clientId, source = StoredMessage.SPOKEN) },
        resume = { connection.resume(it) },
        lastReply = { id -> runCatching { client.messages(id).lastOrNull { it.role != "user" }?.content }.getOrNull() },
        stillRunning = { id -> runCatching { id in client.threads().running }.getOrDefault(false) },
    )
    private val _state = MutableStateFlow(VoiceState())
    val state: StateFlow<VoiceState> = _state

    private var threadId: String? = null
    private var sentAt = 0L
    private var clientId = VoiceTurn.newClientId()
    private var worked = false
    private var job: Job? = null
    /** This sheet's own stop switch for the request it is hearing; dictation for other apps has its own. */
    @Volatile private var hearing = AtomicBoolean(true)

    init {
        // Phase changes with a monotonic clock, for latency runs from adb. Never the words: logcat is not private enough.
        scope.launch {
            _state.map { it.phase }.distinctUntilChanged().collect { Log.i(TAG, "phase=$it at=${SystemClock.elapsedRealtime()}") }
        }
        scope.launch { connection.connected.collect { up -> _state.update { it.copy(reconnecting = !up) } } }
        scope.launch(Dispatchers.IO) { handoff.unseenAnswer()?.let { _state.update { s -> s.copy(afterClose = true) } } }
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
        VoicePhase.MicNeeded, VoicePhase.MicBlocked -> listen()
        VoicePhase.Listening -> hearing.set(true)
        VoicePhase.ModelReady -> listen()
        VoicePhase.Thinking, VoicePhase.LookingUp, VoicePhase.Answering -> stop()
        VoicePhase.Stopped -> listen()
        VoicePhase.Idle -> listen()
        VoicePhase.WaitingForHq -> Unit
    }

    /** Don't send: the question is dropped, the words stay on the sheet and nothing was sent. */
    fun dontSend() {
        job?.cancel()
        _state.update { it.copy(phase = VoicePhase.Idle, problem = null) }
    }

    /** Send again: the words that were heard go out once more, as a new request. */
    fun sendAgain() {
        val words = _state.value.heard.takeIf { it.isNotBlank() } ?: return
        run { ask(words) }
    }

    /** The owner opened the chat: the next-open line has done its job. */
    fun chatOpened() = handoff.markSeen()

    /** Stop only stops: HQ is told, nothing listens, and the next tap asks again. */
    private fun stop() {
        interrupt()
        _state.update { it.copy(phase = VoicePhase.Stopped) }
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
            } catch (e: Ears.Busy) {
                refuse(words.s(R.string.voice_mic_busy))
            } catch (e: Exception) {
                Log.w(TAG, "voice turn failed", e)
                refuse(words.s(R.string.voice_engine_failed))
            }
        }
    }

    /** Android's dialog answered "Don't allow" once: the button becomes Allow and Type instead is offered. */
    fun micNeeded() = _state.update { it.copy(phase = VoicePhase.MicNeeded, problem = null) }

    /** Android will not ask again: the sheet says so and points at App info. [still] adds the line for a return with the microphone still off. */
    fun micBlocked(still: Boolean = false) = _state.update { it.copy(phase = VoicePhase.MicBlocked, problem = if (still) words.s(R.string.voice_mic_still_off) else null) }

    fun refuse(problem: String) = _state.update { it.copy(phase = VoicePhase.Idle, problem = problem) }

    /** The sheet left the screen: stop hearing, and stop HQ's answer if one is running. */
    fun quiet() {
        val phase = _state.value.phase
        // Closing keeps HQ working unless the question just went out and no tool has started: then it was a slip.
        val slip = !worked && SystemClock.elapsedRealtime() - sentAt < SLIP_MS
        val working = phase == VoicePhase.Thinking || phase == VoicePhase.LookingUp || phase == VoicePhase.Answering
        if (phase != VoicePhase.Idle) { if (slip) interrupt() else { job?.cancel(); hearing.set(true) } }
        // HQ keeps working: one watcher takes the question over and says so once, quietly, when it ends.
        if (working && !slip) handoff.adopt(Pending(clientId, threadId, System.currentTimeMillis()))
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
        if (MicAsk.onCall(app.getSystemService(android.media.AudioManager::class.java)?.mode ?: 0)) return refuse(words.s(R.string.voice_on_call))
        _state.update { VoiceState(VoicePhase.Preparing, silent = it.silent, threadId = it.threadId) }
        _state.update { it.copy(heard = "", problem = null, needsOk = false, cold = false, slowLoad = false) }
        val stop = AtomicBoolean(false).also { hearing = it }
        val said = ears.hear(
            stop,
            onRecording = { _state.update { it.copy(phase = VoicePhase.Listening) } },
            onPartial = { partial -> _state.update { it.copy(heard = partial, cold = false, slowLoad = false) } },
            onLoad = { load -> _state.update { it.copy(cold = load != Ears.Load.Ready, slowLoad = load == Ears.Load.Slow) } },
        ).trim()
        _state.update { it.copy(cold = false, slowLoad = false) }
        if (said.isEmpty()) {
            val silent = _state.value.silent + 1
            _state.update { it.copy(silent = silent) }
            return refuse(MicAsk.silenceLine(silent, android.os.Build.VERSION.SDK_INT).let { VoiceWords.silence(words, it) })
        }
        _state.update { it.copy(silent = 0) }
        ask(Invocation.strip(said))
    }

    /** HQ must be up before a question goes: wait for it while it starts, say so at once if it has stopped. */
    private suspend fun ask(question: String) {
        _state.update { it.copy(heard = question, reply = "", problem = null, needsOk = false) }
        if (!connection.connected.value) {
            if (status.value is Connection.Stopped) return refuse(words.s(R.string.voice_fail_hq_stopped))
            _state.update { it.copy(phase = VoicePhase.WaitingForHq) }
            withTimeoutOrNull(WAIT_FOR_HQ_MS) { connection.connected.first { it } } ?: return refuse(words.s(R.string.voice_fail_no_reply))
        }
        _state.update { it.copy(phase = VoicePhase.Thinking) }
        sentAt = SystemClock.elapsedRealtime()
        worked = false
        clientId = VoiceTurn.newClientId()
        turn.ask(question, threadId, clientId).collect(::onOutput)
        _state.update { if (it.phase == VoicePhase.Stopped) it else it.copy(phase = VoicePhase.Idle) }
    }

    private fun onOutput(out: TurnOutput) {
        when (out) {
            is TurnOutput.Thread -> { threadId = out.id; example.hq.spike.ui.chat.SpokenThreads.mark(app, out.id); _state.update { it.copy(threadId = out.id) } }
            is TurnOutput.Text -> _state.update { it.copy(phase = VoicePhase.Answering, reply = it.reply + out.chunk) }
            is TurnOutput.Failed -> _state.update { it.copy(problem = lineFor(out.reason)) }
            TurnOutput.NeedsOk -> _state.update { it.copy(needsOk = true) }
            is TurnOutput.Reset -> _state.update { it.copy(reply = out.text) }
            TurnOutput.Working -> { worked = true; _state.update { it.copy(phase = VoicePhase.LookingUp) } }
            TurnOutput.Done -> Unit
        }
    }

    /** The turn's own failure sentences are catalogue words; anything else is HQ's own reason, shown as it came. */
    private fun lineFor(reason: String) = when (reason) {
        VoiceTurn.HQ_STOPPED -> words.s(R.string.voice_fail_hq_stopped)
        VoiceTurn.RESTARTING -> words.s(R.string.voice_fail_restarting)
        VoiceTurn.NO_REPLY -> words.s(R.string.voice_fail_no_reply)
        VoiceTurn.STILL_WORKING -> words.s(R.string.voice_still_working)
        VoiceTurn.LOST_PART -> words.s(R.string.voice_fail_lost_part)
        VoiceTurn.BUSY -> words.s(R.string.voice_fail_busy)
        VoiceTurn.NOT_ACCEPTED -> words.s(R.string.voice_fail_not_accepted)
        else -> reason
    }

    private fun interrupt() {
        job?.cancel()
        hearing.set(true)
        val phase = _state.value.phase
        if (phase == VoicePhase.Thinking || phase == VoicePhase.LookingUp || phase == VoicePhase.Answering) threadId?.let(connection::stop)
    }

    private companion object {
        const val TAG = "HqVoice"
        const val SLIP_MS = 3_000L
        const val WAIT_FOR_HQ_MS = 30_000L
    }
}
