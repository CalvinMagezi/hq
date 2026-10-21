package example.hq.spike.ui.settings

import android.app.Application
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.PowerManager
import android.provider.Settings
import androidx.lifecycle.AndroidViewModel
import example.hq.spike.AlertPoster
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import java.io.File
import java.time.Instant
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONObject

data class SettingsUi(
    val open: Boolean = false,
    val stack: List<SettingsPage> = emptyList(),
    val snap: SettingsSnapshot = SettingsSnapshot(),
    val loaded: Boolean = false,
    val phone: List<HomeRow> = emptyList(),
    val safetyTab: SettingsWords.SafetyTab = SettingsWords.SafetyTab.Safety,
    val mode: String = "standby",
    val build: String = "",
    val line: String? = null,
    val now: Instant = Instant.now(),
) {
    val page: SettingsPage get() = stack.lastOrNull() ?: SettingsPage.Home
}

class SettingsActions(
    val onOpen: () -> Unit = {},
    val onBack: () -> Unit = {},
    val onPage: (SettingsPage) -> Unit = {},
    val onTab: (SettingsWords.SafetyTab) -> Unit = {},
    val onPhoneRow: (String) -> Unit = {},
    val onCopyDiagnostics: () -> Unit = {},
    val onDismissLine: () -> Unit = {},
    /** One read of everything Settings shows; the page calls it every few seconds while it is showing and the app is in front. */
    val onRefresh: suspend () -> Unit = {},
)

class SettingsViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val _state = MutableStateFlow(SettingsUi(build = buildText(app)))
    val state: StateFlow<SettingsUi> = _state
    private var frozen = false

    val actions = SettingsActions(
        onOpen = ::open,
        onBack = { _state.update { if (it.stack.isEmpty()) it.copy(open = false) else it.copy(stack = it.stack.dropLast(1)) } },
        onPage = { p -> _state.update { it.copy(stack = it.stack + p) } },
        onTab = { t -> _state.update { it.copy(safetyTab = t) } },
        onPhoneRow = ::phoneRow,
        onCopyDiagnostics = ::copyDiagnostics,
        onDismissLine = { _state.update { it.copy(line = null) } },
        onRefresh = ::refreshNow,
    )

    /** Closes Settings and every page inside it. */
    fun close() = _state.update { it.copy(open = false, stack = emptyList()) }

    fun freeze(fixed: SettingsUi) {
        frozen = true
        _state.value = fixed
    }

    /** Opens Settings on its home page. What is on it comes from [refreshNow], which the page runs while it is showing. */
    private fun open() {
        _state.update { it.copy(open = true, stack = emptyList(), line = null, phone = if (frozen) it.phone else phoneRows(), mode = if (frozen) it.mode else modeOf(), now = Instant.now()) }
    }

    /** Reads every route at once, then what Android says about HQ; one route that fails shows as such on its own row only. */
    suspend fun refreshNow() {
        if (frozen) return
        val routes = listOf("settings" to "/api/settings", "chatApps" to "/api/chat-apps", "connector" to "/api/oauth/connector", "devices" to "/api/auth/devices", "budgets" to "/api/budgets", "health" to "/health")
        val got = coroutineScope { routes.map { (key, path) -> async { key to get(path) } }.awaitAll() }.toMap()
        if (frozen) return
        fun body(key: String) = got[key]?.body
        val snap = SettingsSnapshot(
            body("settings"), body("chatApps"), body("connector"), body("devices"), body("budgets"), body("health"),
            unavailable = got.filterValues { it.body == null && it.answered }.keys,
        )
        _state.update { it.copy(snap = snap, loaded = true, phone = phoneRows(), mode = modeOf(), now = Instant.now()) }
    }

    /** What one route gave back: its JSON, or none; and whether HQ answered at all (an error answer is not the same as no answer). */
    private class Got(val body: JSONObject?, val answered: Boolean)

    private suspend fun get(path: String): Got = try { Got(client.json(path), true) } catch (e: HqUnavailable) { Got(null, e.message?.startsWith("HTTP ") == true) }

    private fun ctx(): Application = getApplication()

    /** "hub · build 1": the version name and the build number, as the board writes them. */
    private fun buildText(app: Application): String {
        val info = app.packageManager.getPackageInfo(app.packageName, 0)
        @Suppress("DEPRECATION")
        val number = if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.P) info.longVersionCode else info.versionCode.toLong()
        return "${info.versionName.orEmpty().ifBlank { "hub" }} \u00b7 build $number"
    }

    private fun modeOf(): String = try { File(ctx().filesDir, "mode").takeIf { it.length() <= MODE_MAX }?.readText()?.trim().orEmpty().ifEmpty { "standby" } } catch (e: java.io.IOException) { "standby" }

    /** The rows under "This phone": what Android says about HQ itself, read here rather than from the server. */
    private fun phoneRows(): List<HomeRow> {
        val c = ctx()
        val active = modeOf() == "active"
        val battery = (c.getSystemService(Context.POWER_SERVICE) as PowerManager).isIgnoringBatteryOptimizations(c.packageName)
        val nm = c.getSystemService(NotificationManager::class.java)
        fun on(id: String) = nm.getNotificationChannel(id)?.importance?.let { it != NotificationManager.IMPORTANCE_NONE } ?: true
        val allowed = nm.areNotificationsEnabled()
        return listOf(
            HomeRow(null, "background", "Background work", if (active) "On · answers chats, runs tasks" else "Only when the app is open"),
            HomeRow(null, "battery", "Keep running", if (battery) "Good · 1 thing to check" else "Battery use is limited", warn = !battery),
            HomeRow(null, "notifications", "Notifications", if (!allowed) "Off in Android" else "Needs you ${if (on(AlertPoster.NEEDS)) "on" else "off"} · Updates ${if (on(AlertPoster.UPDATES)) "on" else "off"}", warn = !allowed),
            HomeRow(null, "capture", "Capture and widgets", "Sharing opens a new note"),
        )
    }

    private fun phoneRow(key: String) {
        val c = ctx()
        val intent = when (key) {
            "battery" -> Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
            "notifications" -> Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, c.packageName)
            else -> null
        }
        if (intent == null) {
            _state.update { it.copy(line = if (key == "background") "Switching background work is not in the native app yet." else "These options are not in the native app yet.") }
            return
        }
        try { c.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } catch (e: android.content.ActivityNotFoundException) { _state.update { it.copy(line = "Android could not open that screen.") } }
    }

    private fun copyDiagnostics() {
        val s = _state.value
        val text = buildString {
            appendLine("AgentHQ hub build ${s.build}")
            appendLine("Run mode: ${s.mode}")
            appendLine("Health: ${s.snap.health?.toString() ?: "no answer"}")
        }
        val cm = ctx().getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
        cm.setPrimaryClip(android.content.ClipData.newPlainText("HQ diagnostics", text))
        _state.update { it.copy(line = "Copied.") }
    }

    private companion object {
        const val MODE_MAX = 64L
    }
}
