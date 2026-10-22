package example.hq.spike.ui.connect

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.HqServices
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

data class ConnectUi(val facts: ConnectFacts? = null, val line: String? = null, val busy: Boolean = false)

class ConnectViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val mutable = MutableStateFlow(ConnectUi())
    val ui: StateFlow<ConnectUi> = mutable.asStateFlow()

    init {
        viewModelScope.launch {
            while (isActive) {
                refresh()
                delay(POLL_MS)
            }
        }
    }

    private suspend fun refresh() {
        val facts = try {
            facts(client.connectorStatus())
        } catch (e: Exception) {
            // hq is starting or stopped: show the stand-in address if there is one and say nothing is on.
            ConnectFacts(url = standIn(), source = standIn()?.let { UrlSource.Stub })
        }
        mutable.update { it.copy(facts = facts) }
    }

    private fun facts(status: JSONObject): ConnectFacts {
        val enabled = status.optBoolean("enabled")
        val hubUrl = status.optString("url").takeIf { enabled && it.isNotBlank() }
        val stub = if (hubUrl == null) standIn() else null
        return ConnectFacts(
            enabled = enabled,
            url = hubUrl ?: stub,
            source = if (hubUrl != null) UrlSource.Hub else stub?.let { UrlSource.Stub },
            armedUntil = if (status.isNull("armed_until")) null else status.optLong("armed_until"),
            clients = status.optJSONArray("grants")?.length() ?: 0,
        )
    }

    /** Until the edge exists the address can be put in files/connector_url to see the screen with one. */
    private fun standIn(): String? = ConnectWords.standIn(runCatching { File(getApplication<Application>().filesDir, STAND_IN_FILE).takeIf { it.isFile && it.length() < MAX_FILE }?.readText() }.getOrNull())

    fun arm() = act("Could not arm the connector.") { client.armConnector(ConnectWords.ARM_MINUTES) }

    fun disarm() = act("Could not close the window.") { client.disarmConnector() }

    private fun act(failure: String, call: suspend () -> Unit) {
        mutable.update { it.copy(busy = true, line = null) }
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) { call() }
                refresh()
                mutable.update { it.copy(busy = false) }
            } catch (e: Exception) {
                mutable.update { it.copy(busy = false, line = failure) }
            }
        }
    }

    fun dismissLine() = mutable.update { it.copy(line = null) }

    private companion object {
        const val POLL_MS = 5_000L
        const val STAND_IN_FILE = "connector_url"
        const val MAX_FILE = 1_024L
    }
}
