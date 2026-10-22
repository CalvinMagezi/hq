package example.hq.spike.ui.usage

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import org.json.JSONObject

/** The balance chip in the top bar: what is left on the AI service HQ uses, read once a minute. */
class UsageViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val _chip = MutableStateFlow<BalanceChip?>(null)
    val chip: StateFlow<BalanceChip?> = _chip
    private var frozen = false
    private var lastSettings: JSONObject? = null

    init {
        viewModelScope.launch {
            while (true) {
                if (!frozen) refresh()
                delay(POLL_MS)
            }
        }
    }

    fun freeze(fixed: BalanceChip?) {
        frozen = true
        _chip.value = fixed
    }

    suspend fun refresh() {
        val routes = listOf("/api/settings", "/api/usage/providers", "/api/copilot-usage", "/api/budgets")
        val got = coroutineScope { routes.map { path -> async { get(path) } }.awaitAll() }
        if (frozen || got.all { it == null }) return
        // A poll where the settings did not answer keeps the order HQ was last seen with, so the chip does not switch service for a minute.
        val settings = got[0] ?: lastSettings
        lastSettings = settings
        _chip.value = UsageWords.chip(UsageSnapshot(settings, got[1], got[2], got[3]))
    }

    private suspend fun get(path: String): JSONObject? = try { client.json(path) } catch (e: HqUnavailable) { null }

    private companion object {
        const val POLL_MS = 60_000L
    }
}
