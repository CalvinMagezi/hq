package example.hq.spike.data

import android.content.Context
import android.content.Intent
import example.hq.spike.HubService
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
}

/** The parts of the app that live as long as the process: the client, its socket and the health status. */
class HqServices private constructor(app: Context) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val client = HqClient(token = { HubService.token(app) })
    val connection = ChatConnection(client, scope)

    private val _status = MutableStateFlow<HqStatus>(HqStatus.Starting(0))
    val status: StateFlow<HqStatus> = _status

    init {
        scope.launch {
            var downSince = System.currentTimeMillis()
            while (true) {
                if (client.healthy()) {
                    _status.value = HqStatus.Live
                    downSince = System.currentTimeMillis()
                    delay(LIVE_POLL_MS)
                } else {
                    _status.value = HqStatus.Starting((System.currentTimeMillis() - downSince) / MS_PER_SECOND)
                    delay(DOWN_POLL_MS)
                }
            }
        }
    }

    companion object {
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
