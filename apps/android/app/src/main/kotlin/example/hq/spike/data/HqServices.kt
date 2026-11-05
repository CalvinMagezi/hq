package example.hq.spike.data

import android.content.Context
import android.content.Intent
import example.hq.spike.HubService
import example.hq.spike.ui.startup.FirstLaunch
import example.hq.spike.ui.startup.StartRules
import kotlinx.coroutines.withContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/** What the status line under a screen's name says about the local hq. */
sealed interface HqStatus {
    data object Live : HqStatus
    /** [step] is 1 to 4 (the four steps of a start); [usualSeconds] is what this phone usually takes, once it has two starts to go on. */
    data class Starting(val seconds: Long, val step: Int = 1, val usualSeconds: Long? = null) : HqStatus
    data class Restarting(val seconds: Long) : HqStatus

    /** hq's process ended, or its start has not moved for max(60 s, twice the usual): a fact, not a timer. */
    data class Stopped(val sinceMs: Long, val step: Int?) : HqStatus

    /** hq is up but the chat socket dropped; what is on screen is what was last saved, [age] ago ("1m", "<1m"). */
    data class Saved(val age: String) : HqStatus
}

/** The parts of the app that live as long as the process: the client, its socket and the health status. */
class HqServices private constructor(private val app: Context) {
    /** When HQ last answered, or when the wait began: the seconds the start screens show count from here. */
    @Volatile private var downSince = System.currentTimeMillis()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val client = HqClient(token = { HubService.token(app) }, onSignedOut = { HubService.recoverToken(app) })

    /** For the thread that polls HQ: a call that is accepted and never answered ends after a few seconds instead of holding up the alerts. */
    val background = HqClient(token = { HubService.token(app) }, onSignedOut = { HubService.recoverToken(app) }, callSeconds = BACKGROUND_CALL_SECONDS)
    val connection = ChatConnection(client, scope)

    private val _status = MutableStateFlow<HqStatus>(HqStatus.Starting(0))
    val status: StateFlow<HqStatus> = _status

    /** Free bytes for the app's files while HQ is not answering, for the almost-full line. */
    private val _freeBytes = MutableStateFlow<Long?>(null)
    val freeBytes: StateFlow<Long?> = _freeBytes

    private var sawDown = false
    private var stepFloor = 0
    private var stepMovedAt = System.currentTimeMillis()

    init {
        scope.launch(Dispatchers.IO) { client.warmUp() }
        scope.launch {
            while (true) {
                if (client.healthy()) {
                    if (sawDown) StartRules.recordStart(app, System.currentTimeMillis() - downSince)
                    sawDown = false
                    stepFloor = 0
                    _status.value = HqStatus.Live
                    _freeBytes.value = null
                    downSince = System.currentTimeMillis()
                    stepMovedAt = downSince
                    FirstLaunch.markDone(app)
                    if (downSince - StartRules.lastAnswer(app) >= ANSWER_MARK_MS) StartRules.markAnswer(app, downSince)
                    delay(LIVE_POLL_MS)
                } else {
                    sawDown = true
                    _status.value = withContext(Dispatchers.IO) { probe() }
                    delay(DOWN_POLL_MS)
                }
            }
        }
    }

    /** Reads the four steps once: the hub service, hq's process, its port. */
    private fun probe(): HqStatus {
        val now = System.currentTimeMillis()
        val alive = HubService.hqAlive()
        val raw = StartRules.step(HubService.hubUp(), alive, alive && portOpen())
        if (!alive) stepFloor = 0
        val step = maxOf(raw, stepFloor)
        if (step != stepFloor) stepMovedAt = now
        stepFloor = step
        _freeBytes.value = freeSpace()
        val usual = StartRules.usual(StartRules.starts(app))
        val decided = ConnectionRules.decide(
            ConnectionInputs(
                nowMs = now, processAlive = alive || HubService.lastExit(app) == null, restarting = HubService.restarting(),
                startStep = step - 1, startedAtMs = downSince, stepMovedAtMs = stepMovedAt, usualStartMs = usual?.times(MS_PER_SECOND),
                serverSignedOut = false, serverUnreachableSinceMs = null, socketDownSinceMs = null, offlineSinceMs = null, aiFailingService = null, aiFailingAtMs = null,
            ),
        )
        return when (decided) {
            is Connection.Stopped -> HqStatus.Stopped(decided.sinceMs, decided.step?.plus(1))
            is Connection.Restarting -> HqStatus.Restarting(decided.seconds)
            is Connection.Starting -> HqStatus.Starting(decided.seconds, step, decided.usualSeconds)
            else -> HqStatus.Starting((now - downSince) / MS_PER_SECOND, step, usual)
        }
    }

    private fun portOpen(): Boolean = try {
        java.net.Socket().use { it.connect(java.net.InetSocketAddress("127.0.0.1", example.hq.spike.BuildConfig.HQ_PORT), PORT_TIMEOUT_MS) }
        true
    } catch (e: java.io.IOException) { false }

    /** What Android would let the app write, counting what it can clear on its own; the app's own files folder if that is not known. */
    private fun freeSpace(): Long = try {
        val sm = app.getSystemService(android.os.storage.StorageManager::class.java)
        sm.getAllocatableBytes(sm.getUuidForPath(app.filesDir))
    } catch (e: java.io.IOException) { app.filesDir.usableSpace }

    /** "Try again": the wait starts over from now. */
    fun restartWait() {
        downSince = System.currentTimeMillis()
        stepFloor = 0
        stepMovedAt = downSince
        _status.value = HqStatus.Starting(0)
    }

    companion object {
        private const val BACKGROUND_CALL_SECONDS = 10L
        private const val LIVE_POLL_MS = 5_000L
        private const val DOWN_POLL_MS = 1_000L
        private const val ANSWER_MARK_MS = 60_000L
        private const val PORT_TIMEOUT_MS = 300
        private const val MS_PER_SECOND = 1_000L
        @Volatile private var instance: HqServices? = null

        fun get(context: Context): HqServices = instance ?: synchronized(this) {
            instance ?: HqServices(context.applicationContext).also { instance = it }
        }

        /** The hub service is what runs hq. Every screen that can be the first one opened starts it, or nothing answers. */
        fun ensureHubRunning(context: Context) {
            context.startForegroundService(Intent(context, HubService::class.java))
        }
    }
}
