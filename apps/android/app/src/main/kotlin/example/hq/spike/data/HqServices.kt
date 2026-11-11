package example.hq.spike.data

import android.content.Context
import android.content.Intent
import example.hq.spike.HubService
import example.hq.spike.ServerHq
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

/** The parts of the app that live as long as the process: the client, its socket and the health status. */
class HqServices private constructor(private val app: Context) {
    /** When HQ last answered, or when the wait began: the seconds the start screens show count from here. */
    @Volatile private var downSince = System.currentTimeMillis()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val client = HqClient(token = { HubService.token(app) }, onSignedOut = { HubService.recoverToken(app) })

    /** For the thread that polls HQ: a call that is accepted and never answered ends after a few seconds instead of holding up the alerts. */
    val background = HqClient(token = { HubService.token(app) }, onSignedOut = { HubService.recoverToken(app) }, callSeconds = BACKGROUND_CALL_SECONDS)
    val connection = ChatConnection(client, scope)

    /** Saves calendar events HQ asks for; it listens on the same socket for as long as the process lives. */
    private val calendarPrefs = example.hq.spike.calendar.CalendarPrefs(app)
    val calendarStore = example.hq.spike.calendar.AndroidCalendarStore(app)
    private val deviceActions = example.hq.spike.calendar.DeviceActionHandler(
        example.hq.spike.calendar.HqDeviceActionLink(client),
        example.hq.spike.calendar.CalendarCreator(calendarStore, calendarPrefs, calendarPrefs),
    )

    private val _status = MutableStateFlow<Connection>(Connection.Starting(0, 0, null))

    /** The one Connection state every place reads, decided once here by [ConnectionRules.decide]. */
    val status: StateFlow<Connection> = _status

    private val network = NetworkWatch(app)

    @Volatile private var healthy = false
    @Volatile private var aiFailure: AiStatus? = null
    @Volatile private var lastProbe: ProbeFacts? = null

    /** What the last look at a down HQ found, kept so the state can be decided again each second without another probe. */
    private data class ProbeFacts(val alive: Boolean, val step: Int, val usualMs: Long?)

    /** Free bytes for the app's files while HQ is not answering, for the almost-full line. */
    private val _freeBytes = MutableStateFlow<Long?>(null)
    val freeBytes: StateFlow<Long?> = _freeBytes

    private var sawDown = false
    private var stepFloor = 0
    private var stepMovedAt = System.currentTimeMillis()

    init {
        deviceActions.start(scope, connection)
        scope.launch(Dispatchers.IO) { client.warmUp() }
        scope.launch {
            while (true) {
                if (client.healthy()) {
                    if (sawDown) StartRules.recordStart(app, System.currentTimeMillis() - downSince)
                    sawDown = false
                    stepFloor = 0
                    healthy = true
                    lastProbe = null
                    _freeBytes.value = null
                    downSince = System.currentTimeMillis()
                    stepMovedAt = downSince
                    FirstLaunch.markDone(app)
                    if (downSince - StartRules.lastAnswer(app) >= ANSWER_MARK_MS) StartRules.markAnswer(app, downSince)
                    recompute()
                    delay(LIVE_POLL_MS)
                } else {
                    sawDown = true
                    healthy = false
                    withContext(Dispatchers.IO) { probe() }
                    recompute()
                    delay(DOWN_POLL_MS)
                }
            }
        }
        // The 3 s offline wait and the 5 s chat-link wait have no event of their own, so the state is decided again every second.
        scope.launch { while (true) { delay(DOWN_POLL_MS); recompute() } }
    }

    /** Reads the four steps once: the hub service, hq's process, its port. */
    private fun probe() {
        val now = System.currentTimeMillis()
        val alive = HubService.hqAlive()
        val raw = StartRules.step(HubService.hubUp(), alive, alive && portOpen())
        if (!alive) stepFloor = 0
        val step = maxOf(raw, stepFloor)
        if (step != stepFloor) stepMovedAt = now
        stepFloor = step
        _freeBytes.value = freeSpace()
        lastProbe = ProbeFacts(alive || HubService.lastExit(app) == null, step, StartRules.usual(StartRules.starts(app))?.times(MS_PER_SECOND))
    }

    /** The AI state HQ reported on its signed-in route; a good answer clears it. Read when the sheet opens and after a failed turn. */
    suspend fun refreshAi() {
        aiFailure = client.aiStatus()?.takeIf { !it.answering }
        recompute()
    }

    /** Decides the one state from everything the phone knows now. The first true row wins (rules section 1). */
    fun recompute() {
        val now = System.currentTimeMillis()
        val probe = lastProbe
        if (connection.connected.value || !healthy) socketDownSince = 0L else if (socketDownSince == 0L) socketDownSince = now
        val socketDown = socketDownSince
        _status.value = ConnectionRules.decide(
            ConnectionInputs(
                nowMs = now, processAlive = if (healthy) true else probe?.alive ?: true, restarting = !healthy && HubService.restarting(),
                startStep = if (healthy) null else (probe?.step ?: 1) - 1, startedAtMs = downSince, stepMovedAtMs = if (healthy) null else stepMovedAt,
                usualStartMs = probe?.usualMs ?: StartRules.usual(StartRules.starts(app))?.times(MS_PER_SECOND),
                serverSignedOut = ServerHq.signedOut(app) && ServerHq.address(app) != null,
                serverUnreachableSinceMs = ServerHq.unreachableSince(app),
                socketDownSinceMs = socketDown.takeIf { it > 0 && now - it >= SOCKET_GRACE_MS },
                offlineSinceMs = network.offlineSince.value,
                aiFailingService = aiFailure?.failingService, aiFailingAtMs = aiFailure?.failedAtMs,
            ),
        )
    }

    @Volatile private var socketDownSince = 0L

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
        lastProbe = null
        recompute()
    }

    companion object {
        private const val BACKGROUND_CALL_SECONDS = 10L
        private const val LIVE_POLL_MS = 5_000L
        private const val DOWN_POLL_MS = 1_000L
        private const val ANSWER_MARK_MS = 60_000L
        private const val PORT_TIMEOUT_MS = 300
        private const val SOCKET_GRACE_MS = 5_000L
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
