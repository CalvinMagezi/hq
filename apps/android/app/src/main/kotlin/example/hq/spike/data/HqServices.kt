package example.hq.spike.data

import android.content.Context
import android.content.Intent
import example.hq.spike.HubService
import example.hq.spike.ui.startup.FirstLaunch
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
    data class Starting(val seconds: Long) : HqStatus

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

    init {
        scope.launch(Dispatchers.IO) { client.warmUp() }
        scope.launch {
            while (true) {
                if (client.healthy()) {
                    _status.value = HqStatus.Live
                    downSince = System.currentTimeMillis()
                    FirstLaunch.markDone(app)
                    delay(LIVE_POLL_MS)
                } else {
                    _status.value = HqStatus.Starting((System.currentTimeMillis() - downSince) / MS_PER_SECOND)
                    delay(DOWN_POLL_MS)
                }
            }
        }
    }

    /** "Try again": the wait starts over from now. */
    fun restartWait() {
        downSince = System.currentTimeMillis()
        _status.value = HqStatus.Starting(0)
    }

    companion object {
        private const val BACKGROUND_CALL_SECONDS = 10L
        private const val LIVE_POLL_MS = 5_000L
        private const val DOWN_POLL_MS = 1_000L
        private const val MS_PER_SECOND = 1_000L
        @Volatile private var instance: HqServices? = null

        fun get(context: Context): HqServices = instance ?: synchronized(this) {
            instance ?: HqServices(context.applicationContext).also { instance = it }
        }

        /** The hub service is what runs hq. MainActivity starts it today; the shell must too, or nothing answers. */
        fun ensureHubRunning(context: Context) {
            context.startForegroundService(Intent(context, HubService::class.java))
        }
    }
}
