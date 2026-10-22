package example.hq.spike.ui.settings

import android.app.ActivityManager
import android.app.Application
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.os.SystemClock
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.AlertPoster
import example.hq.spike.HubService
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import java.io.File
import java.time.Instant
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
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
    /** The run mode the owner is being asked to confirm (true is Active), or null when no dialog is open. */
    val askMode: Boolean? = null,
    /** hq is restarting for a new run mode; the choices are off until it is back. */
    val restarting: Boolean = false,
    /** What Android says about keeping AgentHQ running and about its notifications; read again whenever Settings is refreshed. */
    val keep: KeepFacts = KeepFacts(),
    val notif: NotifFacts = NotifFacts(),
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
    val onMode: (Boolean) -> Unit = {},
    val onModeCancel: () -> Unit = {},
    val onModeConfirm: () -> Unit = {},
    val onFix: (PhoneFix) -> Unit = {},
    val onChannel: (String) -> Unit = {},
    val onAnswers: (Boolean) -> Unit = {},
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
        onMode = { toActive -> _state.update { if (it.restarting || toActive == (it.mode == "active")) it else it.copy(askMode = toActive) } },
        onModeCancel = { _state.update { it.copy(askMode = null) } },
        onModeConfirm = ::confirmMode,
        onFix = ::fix,
        onChannel = ::openChannel,
        onAnswers = { on -> AlertPoster.setAnswersAllowed(ctx(), on); _state.update { it.copy(notif = it.notif.copy(answers = on)) } },
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
        _state.update { it.copy(open = true, stack = emptyList(), line = null, phone = if (frozen) it.phone else phoneRows(), mode = if (frozen) it.mode else modeOf(), now = Instant.now()).withFacts() }
    }

    /**
     * Reads every route at once. The six local ones are shown as soon as they answer; the three that ask a provider or add up a month (which can
     * take many seconds) are added when they arrive. One route that fails shows as such on its own row only.
     */
    suspend fun refreshNow() {
        if (frozen) return
        val local = listOf("settings" to "/api/settings", "chatApps" to "/api/chat-apps", "connector" to "/api/oauth/connector", "devices" to "/api/auth/devices", "budgets" to "/api/budgets", "health" to "/health")
        val remote = listOf("providers" to "/api/usage/providers", "copilot" to "/api/copilot-usage", "forecast" to "/api/usage/forecast")
        coroutineScope {
            val slow = remote.map { (key, path) -> async { key to get(path) } }
            val fast = local.map { (key, path) -> async { key to get(path) } }.awaitAll().toMap()
            if (frozen) return@coroutineScope
            val before = _state.value.snap
            publish(fast, mapOf("providers" to Got(before.providers, before.providers != null), "copilot" to Got(before.copilot, before.copilot != null), "forecast" to Got(before.forecast, before.forecast != null)))
            val late = slow.awaitAll().toMap()
            if (frozen) return@coroutineScope
            publish(fast, late)
        }
    }

    private fun publish(fast: Map<String, Got>, late: Map<String, Got>) {
        val got = fast + late
        fun body(key: String) = got[key]?.body
        val snap = SettingsSnapshot(
            body("settings"), body("chatApps"), body("connector"), body("devices"), body("budgets"), body("health"),
            unavailable = got.filterValues { it.body == null && it.answered }.keys,
            providers = body("providers"), copilot = body("copilot"), forecast = body("forecast"),
        )
        _state.update { it.copy(snap = snap, loaded = true, phone = phoneRows(), mode = modeOf(), now = Instant.now()).withFacts() }
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
            HomeRow(SettingsPage.Background, "background", "Background work", if (active) SettingsWords.BACKGROUND_ON else SettingsWords.BACKGROUND_OFF),
            HomeRow(SettingsPage.KeepRunning, "battery", "Keep running", PhoneWords.keepSummary(keepFacts(), Instant.now()), warn = keepFacts().let { PhoneWords.keepRunning(it, Instant.now()).checked.any { r -> !r.ok } }),
            HomeRow(SettingsPage.Notifications, "notifications", "Notifications", if (!allowed) "Off in Android" else "Needs you ${if (on(AlertPoster.NEEDS)) "on" else "off"} · Updates ${if (on(AlertPoster.UPDATES)) "on" else "off"}", warn = !allowed),
            HomeRow(null, "capture", "Capture and widgets", "Sharing opens a new note"),
        )
    }

    private fun SettingsUi.withFacts(): SettingsUi = if (frozen) this else copy(keep = keepFacts(), notif = notifFacts())

    /** What Android says about this app staying alive: the battery setting, permission auto-removal, notifications, and the last time it stopped it. */
    private fun keepFacts(): KeepFacts {
        val c = ctx()
        val pkg = c.packageName
        val sdk = Build.VERSION.SDK_INT
        val am = c.getSystemService(ActivityManager::class.java)
        val last = if (sdk >= Build.VERSION_CODES.R) {
            am.getHistoricalProcessExitReasons(pkg, 0, EXIT_SCAN).firstOrNull { PhoneWords.androidStopped(it.reason) }?.let { LastStop(it.reason, Instant.ofEpochMilli(it.timestamp)) }
        } else null
        return KeepFacts(
            batteryUnrestricted = (c.getSystemService(Context.POWER_SERVICE) as PowerManager).isIgnoringBatteryOptimizations(pkg),
            backgroundRestricted = sdk >= Build.VERSION_CODES.P && am.isBackgroundRestricted,
            autoRevokeOff = if (sdk >= Build.VERSION_CODES.R) c.packageManager.isAutoRevokeWhitelisted else null,
            notificationsAllowed = c.getSystemService(NotificationManager::class.java).areNotificationsEnabled(),
            samsung = Build.MANUFACTURER.equals("samsung", ignoreCase = true),
            lastStop = last,
        )
    }

    private fun notifFacts(): NotifFacts {
        val nm = ctx().getSystemService(NotificationManager::class.java)
        val channels = listOf(AlertPoster.NEEDS, AlertPoster.UPDATES, HUB_CHANNEL).mapNotNull { nm.getNotificationChannel(it) }
            .map { ChannelFacts(it.id, it.name.toString(), it.importance, it.sound != null) }
        return NotifFacts(nm.areNotificationsEnabled(), channels, AlertPoster.answersAllowed(ctx()))
    }

    /** Opens the Android page that changes a setting the owner was told about; a page this phone does not have is tried past, never a crash. */
    private fun fix(what: PhoneFix) {
        val c = ctx()
        val details = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${c.packageName}"))
        val tries = when (what) {
            PhoneFix.Battery -> listOf(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS), details)
            PhoneFix.AutoRevoke -> listOf(Intent(Intent.ACTION_AUTO_REVOKE_PERMISSIONS, Uri.parse("package:${c.packageName}")), details)
            PhoneFix.Notifications -> listOf(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, c.packageName), details)
            PhoneFix.SleepingApps -> listOf(Intent().setClassName(SAMSUNG_BATTERY_PACKAGE, SAMSUNG_BATTERY_PAGE), Intent(Settings.ACTION_BATTERY_SAVER_SETTINGS), details)
        }
        startFirst(tries)
    }

    private fun openChannel(id: String) {
        val c = ctx()
        startFirst(listOf(Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, c.packageName).putExtra(Settings.EXTRA_CHANNEL_ID, id)))
    }

    private fun startFirst(tries: List<Intent>) {
        for (i in tries) {
            try { ctx().startActivity(i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); return } catch (e: android.content.ActivityNotFoundException) { continue } catch (e: SecurityException) { continue }
        }
        _state.update { it.copy(line = "Android could not open that screen.") }
    }

    private fun phoneRow(key: String) {
        val c = ctx()
        val intent = when (key) {
            "battery" -> Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
            "notifications" -> Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, c.packageName)
            else -> null
        }
        if (intent == null) {
            _state.update { it.copy(line = "These options are not in the native app yet.") }
            return
        }
        try { c.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } catch (e: android.content.ActivityNotFoundException) { _state.update { it.copy(line = "Android could not open that screen.") } }
    }

    /**
     * Writes the new run mode and restarts hq, then waits for it to answer again. Both cards stay off meanwhile; the mode shown
     * afterwards is what the file says, not what was tapped.
     */
    private fun confirmMode() {
        val toActive = _state.value.askMode ?: return
        if (_state.value.restarting) return
        _state.update { it.copy(askMode = null, restarting = true) }
        viewModelScope.launch {
            val ok = withContext(Dispatchers.IO) { switchAndWait(toActive) }
            _state.update { it.copy(restarting = false, mode = modeOf(), phone = phoneRows(), line = if (ok) null else RESTART_TIMEOUT_LINE).withFacts() }
            refreshNow()
        }
    }

    private suspend fun switchAndWait(toActive: Boolean): Boolean {
        val before = startStamp().second
        try { HubService.switchMode(ctx(), toActive) } catch (e: java.io.IOException) { return false }
        if (!waitForHq(before)) return false
        if (HubService.childActive() == toActive) return true
        // The switch landed while hq was starting with the old mode: restart once more so it reads the file.
        val again = startStamp().second
        HubService.restartHq()
        return waitForHq(again)
    }

    private suspend fun waitForHq(before: String?): Boolean {
        val watch = RestartWatch(before)
        val end = SystemClock.elapsedRealtime() + RestartWatch.TIMEOUT_MS
        while (SystemClock.elapsedRealtime() < end) {
            delay(RestartWatch.POLL_MS)
            val (answered, stamp) = startStamp()
            if (watch.back(answered, stamp)) return true
        }
        return false
    }

    /** Whether hq answered, and the start time it reports (it changes each time hq starts). */
    private suspend fun startStamp(): Pair<Boolean, String?> =
        try { true to client.json("/api/chat-apps").let { o -> if (o.has("started_at") && !o.isNull("started_at")) o.get("started_at").toString() else null } } catch (e: HqUnavailable) { false to null }

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
        const val EXIT_SCAN = 10
        const val HUB_CHANNEL = "hub"
        const val SAMSUNG_BATTERY_PACKAGE = "com.samsung.android.lool"
        const val SAMSUNG_BATTERY_PAGE = "com.samsung.android.sm.battery.ui.BatteryActivity"
        const val RESTART_TIMEOUT_LINE = "HQ did not come back within two minutes."
    }
}
