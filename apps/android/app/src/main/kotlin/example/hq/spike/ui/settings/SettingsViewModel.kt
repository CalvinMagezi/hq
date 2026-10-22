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
import example.hq.spike.capture.CapturePrefs
import example.hq.spike.capture.CaptureWidget
import example.hq.spike.capture.NeedsYouWidget
import example.hq.spike.capture.Outbox
import example.hq.spike.capture.OutboxSender
import example.hq.spike.capture.TasksWidget
import example.hq.spike.capture.Widgets
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

/** A bot token being set up: whether it is being checked or saved, what was found, and what went wrong. The token itself is never kept here. */
data class SetupState(
    val platform: String, val checking: Boolean = false, val saving: Boolean = false, val found: String? = null, val appId: String? = null,
    val intentOff: Boolean = false, val error: String? = null, val saved: Boolean = false,
)

/** A pairing code waiting for the owner's message. */
data class PairState(val platform: String, val code: String, val command: String, val deepLink: String?, val expiresAt: Long, val error: String? = null)

/** A sign-in code for another device, and when it stops working (epoch seconds). */
data class DeviceCode(val code: String, val expiresAt: Long)

/** Which provider the key is for, and whether HQ is checking it or refused it. */
data class KeySheet(val id: String, val name: String, val replace: Boolean, val checking: Boolean = false, val error: String? = null)

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
    /** The owner is being asked to confirm a restart of hq. */
    val askRestart: Boolean = false,
    /** hq is restarting for a new run mode; the choices are off until it is back. */
    val restarting: Boolean = false,
    /** What Android says about keeping AgentHQ running and about its notifications; read again whenever Settings is refreshed. */
    val keep: KeepFacts = KeepFacts(),
    val notif: NotifFacts = NotifFacts(),
    val capture: CaptureFacts = CaptureFacts(),
    /** What the config file and the backup folder say about backups on this phone. */
    val backup: BackupFacts = BackupFacts(),
    /** The sheet that takes a key, while it is open. The key itself is never kept here. */
    val keySheet: KeySheet? = null,
    /** What is being asked or done on the Connect Claude and Other devices pages. */
    val busy: String? = null,
    val askClaudeOff: Boolean = false,
    val askSignOutAll: Boolean = false,
    val deviceCode: DeviceCode? = null,
    val setup: SetupState? = null,
    val pair: PairState? = null,
    val askDisconnect: String? = null,
    val askReplace: String? = null,
) {
    val page: SettingsPage get() = stack.lastOrNull() ?: SettingsPage.Home

    /** A key is being checked or a bot token saved: Back waits, so it does not close the page that work belongs to. */
    fun busyBehind(): Boolean = keySheet?.checking == true || setup?.saving == true
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
    val onRestart: (Boolean) -> Unit = {},
    val onRestartConfirm: () -> Unit = {},
    val onFix: (PhoneFix) -> Unit = {},
    val onChannel: (String) -> Unit = {},
    val onAnswers: (Boolean) -> Unit = {},
    val onShareOpens: (String) -> Unit = {},
    val onSetup: (String) -> Unit = {},
    val onCheckToken: (String) -> Unit = {},
    val onSaveToken: (String) -> Unit = {},
    val onPair: (String, Boolean) -> Unit = { _, _ -> },
    val onPairPoll: suspend () -> Unit = {},
    val onAskDisconnect: (String?) -> Unit = {},
    val onDisconnect: () -> Unit = {},
    val onAskReplace: (String?) -> Unit = {},
    val onArm: () -> Unit = {},
    val onDisarm: () -> Unit = {},
    val onRevoke: (String) -> Unit = {},
    val onClaudeOff: (Boolean) -> Unit = {},
    val onClaudeOffConfirm: () -> Unit = {},
    val onNewCode: () -> Unit = {},
    val onSignOut: (String) -> Unit = {},
    val onSignOutAll: (Boolean) -> Unit = {},
    val onSignOutAllConfirm: () -> Unit = {},
    val onCopy: (String) -> Unit = {},
    val onKeyOpen: (KeySheet) -> Unit = {},
    val onKeyClose: () -> Unit = {},
    val onKeySave: (String) -> Unit = {},
    val onShowTitles: (Boolean) -> Unit = {},
    val onAddWidget: (Int) -> Unit = {},
    val onSendNow: (String) -> Unit = {},
    val onDiscard: (String) -> Unit = {},
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
        onBack = { _state.update { if (it.busyBehind()) it else if (it.stack.isEmpty()) it.copy(open = false) else it.copy(stack = it.stack.dropLast(1)) } },
        onPage = { p -> _state.update { it.copy(stack = it.stack + p) } },
        onTab = { t -> _state.update { it.copy(safetyTab = t) } },
        onPhoneRow = ::phoneRow,
        onCopyDiagnostics = ::copyDiagnostics,
        onDismissLine = { _state.update { it.copy(line = null) } },
        onMode = { toActive -> _state.update { if (it.restarting || toActive == (it.mode == "active")) it else it.copy(askMode = toActive) } },
        onModeCancel = { _state.update { it.copy(askMode = null) } },
        onModeConfirm = ::confirmMode,
        onRestart = { ask -> _state.update { if (it.restarting) it else it.copy(askRestart = ask) } },
        onRestartConfirm = ::confirmRestart,
        onFix = ::fix,
        onChannel = ::openChannel,
        onSetup = { p -> _state.update { it.copy(setup = SetupState(p), stack = it.stack + SettingsPage.SetupApp) } },
        onCheckToken = ::checkToken,
        onSaveToken = ::saveToken,
        onPair = ::pairApp,
        onPairPoll = ::pairPoll,
        onAskDisconnect = { p -> _state.update { if (it.restarting) it else it.copy(askDisconnect = p) } },
        onDisconnect = ::disconnectApp,
        onAskReplace = { p -> _state.update { it.copy(askReplace = p) } },
        onArm = { act("arm", "Could not open the window. HQ is not answering.") { client.postJson("/api/oauth/connector/arm") } },
        onDisarm = { act("disarm", "Could not close it. HQ is not answering.") { client.postJson("/api/oauth/connector/disarm") } },
        onRevoke = { id -> act("revoke:$id", "Could not disconnect it. HQ is not answering.") { client.postJson("/api/oauth/grants/${java.net.URLEncoder.encode(id, "UTF-8")}/revoke") } },
        onClaudeOff = { ask -> _state.update { it.copy(askClaudeOff = ask) } },
        onClaudeOffConfirm = { _state.update { it.copy(askClaudeOff = false) }; act("off", "Could not disconnect Claude. HQ is not answering.") { client.postJson("/api/oauth/connector/off") } },
        onNewCode = ::newCode,
        onSignOut = { id -> act("out:$id", "Could not sign it out. HQ is not answering.") { client.deleteAt("/api/auth/devices/${java.net.URLEncoder.encode(id, "UTF-8")}") } },
        onSignOutAll = { ask -> _state.update { if (it.restarting) it else it.copy(askSignOutAll = ask) } },
        onSignOutAllConfirm = ::signOutAll,
        onCopy = { text -> copyText(text) },
        onKeyOpen = { k -> _state.update { it.copy(keySheet = k) } },
        onKeyClose = { _state.update { if (it.keySheet?.checking == true) it else it.copy(keySheet = null) } },
        onKeySave = ::saveKey,
        onShareOpens = { v -> CapturePrefs.setShareOpens(ctx(), v); _state.update { it.copy(capture = it.capture.copy(shareOpens = v)) } },
        onShowTitles = { on -> CapturePrefs.setShowTitles(ctx(), on); Widgets.renderAll(ctx()); _state.update { it.copy(capture = it.capture.copy(showTitles = on)) } },
        onAddWidget = ::addWidget, onSendNow = ::sendNow, onDiscard = { id -> Outbox.of(ctx()).remove(id); _state.update { it.copy(capture = captureFacts(), phone = phoneRows()) } },
        onAnswers = { on -> AlertPoster.setAnswersAllowed(ctx(), on); _state.update { it.copy(notif = it.notif.copy(answers = on)) } },
        onRefresh = ::refreshNow,
    )

    /** Closes Settings and every page inside it. */
    fun close() = _state.update { it.copy(open = false, stack = emptyList(), askMode = null, askRestart = false, keySheet = null, askClaudeOff = false, askSignOutAll = false, deviceCode = null, setup = null, pair = null, askDisconnect = null, askReplace = null) }

    fun freeze(fixed: SettingsUi) {
        frozen = true
        _state.value = fixed
    }

    /** Opens Settings on its home page. What is on it comes from [refreshNow], which the page runs while it is showing. */
    private fun open() {
        _state.update { it.copy(open = true, stack = emptyList(), line = null, askMode = null, askRestart = false, phone = if (frozen) it.phone else phoneRows(), mode = if (frozen) it.mode else modeOf(), now = Instant.now()).withFacts() }
    }

    /**
     * Reads every route at once. The seven local ones are shown as soon as they answer; the three that ask a provider or add up a month (which can
     * take many seconds) are added when they arrive. One route that fails shows as such on its own row only.
     */
    suspend fun refreshNow() {
        if (frozen) return
        val local = listOf("settings" to "/api/settings", "chatApps" to "/api/chat-apps", "connector" to "/api/oauth/connector", "devices" to "/api/auth/devices", "budgets" to "/api/budgets", "health" to "/health", "vault" to "/api/vault-status")
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
            providers = body("providers"), copilot = body("copilot"), forecast = body("forecast"), vault = body("vault"),
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
            HomeRow(SettingsPage.CaptureWidgets, "capture", "Capture and widgets", PhoneWords.captureSummary(captureFacts())),
        )
    }

    private fun SettingsUi.withFacts(): SettingsUi = if (frozen) this else copy(keep = keepFacts(), notif = notifFacts(), capture = captureFacts(), backup = backupFacts())

    /** What Android says about this app staying alive: the battery setting, permission auto-removal, notifications, and the last time it stopped it. */
    private var keepCache: Pair<Long, KeepFacts>? = null

    /** The same reading for a few seconds, so the several places a refresh asks do not each ask Android. */
    private fun keepFacts(): KeepFacts {
        val now = SystemClock.elapsedRealtime()
        keepCache?.let { (at, facts) -> if (now - at < KEEP_CACHE_MS) return facts }
        return readKeepFacts().also { keepCache = now to it }
    }

    /** Read from the app's own files each time Settings refreshes; the config file is parsed only for its `backup:` block. */
    private fun backupFacts(): BackupFacts {
        val files = ctx().filesDir
        val config = runCatching { BackupWords.parseConfig(java.io.File(files, "hqhome/config.yaml").readText()) }.getOrNull()
        val dir = config?.dir?.let { java.io.File(it) }?.takeIf { runCatching { it.canonicalPath.startsWith(files.canonicalPath + "/") }.getOrDefault(false) } ?: java.io.File(files, "backups")
        val status = runCatching { BackupWords.parseStatus(java.io.File(dir, "backup-status.json").readText()) }.getOrNull()
        val newest = dir.listFiles { f -> f.name.startsWith("hq-backup-") && f.name.endsWith(".hqbak") }?.maxByOrNull { it.lastModified() }
        return BackupFacts(config, status, newest?.lastModified(), newest?.name)
    }

    private fun readKeepFacts(): KeepFacts {
        val c = ctx()
        val pkg = c.packageName
        val am = c.getSystemService(ActivityManager::class.java)
        return KeepFacts(
            batteryUnrestricted = (c.getSystemService(Context.POWER_SERVICE) as PowerManager).isIgnoringBatteryOptimizations(pkg),
            backgroundRestricted = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P && am.isBackgroundRestricted,
            autoRevokeOff = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) c.packageManager.isAutoRevokeWhitelisted else null,
            notificationsAllowed = c.getSystemService(NotificationManager::class.java).areNotificationsEnabled(),
            samsung = Build.MANUFACTURER.equals("samsung", ignoreCase = true),
            lastStop = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) lastAndroidStop(am, pkg) else null,
        )
    }

    /** The newest exit Android chose (not one the owner or the app asked for), from what it keeps about this app's recent exits. */
    @androidx.annotation.RequiresApi(Build.VERSION_CODES.R)
    private fun lastAndroidStop(am: ActivityManager, pkg: String): LastStop? =
        am.getHistoricalProcessExitReasons(pkg, 0, EXIT_SCAN).firstOrNull { PhoneWords.androidStopped(it.reason) }?.let { LastStop(it.reason, Instant.ofEpochMilli(it.timestamp)) }

    /** Sends the key to HQ, which tries it with the provider before keeping it. Nothing about the key is kept or logged here. */
    private fun saveKey(typed: String) {
        val sheet = _state.value.keySheet ?: return
        val key = typed.trim()
        if (key.isEmpty() || sheet.checking) return
        _state.update { it.copy(keySheet = sheet.copy(checking = true, error = null)) }
        viewModelScope.launch {
            try {
                client.setProviderKey(sheet.id, key)
                _state.update { it.copy(keySheet = null, line = SettingsWords.keySaved(sheet.name)) }
                refreshNow()
            } catch (e: HqUnavailable) {
                _state.update { it.copy(keySheet = it.keySheet?.copy(checking = false, error = SettingsWords.keyFailure(e.status))) }
            }
        }
    }

    /** Asks HQ whether a bot token works, without keeping it. The server answers 200 either way; `ok` says which. */
    private fun checkToken(typed: String) {
        val setup = _state.value.setup ?: return
        val token = typed.trim()
        if (token.isEmpty() || setup.checking || setup.saving) return
        _state.update { it.copy(setup = setup.copy(checking = true, error = null, found = null)) }
        viewModelScope.launch {
            val next = try {
                val r = client.postJson("/api/chat-apps/${setup.platform}/verify", JSONObject().put("token", token))
                if (r.optBoolean("ok")) {
                    val name = r.optString("username").takeIf { n -> n.isNotBlank() }
                    setup.copy(checking = false, found = name?.let { n -> if (setup.platform == "telegram") "@$n" else n } ?: "your bot", appId = r.optString("application_id").takeIf { a -> a.isNotBlank() }, intentOff = setup.platform == "discord" && r.has("message_content") && !r.optBoolean("message_content"))
                } else setup.copy(checking = false, error = r.optString("error").takeIf { e -> e.isNotBlank() } ?: SettingsWords.TOKEN_REFUSED)
            } catch (e: HqUnavailable) {
                setup.copy(checking = false, error = SettingsWords.chatAppFailure(e.status, null))
            }
            _state.update { if (it.setup == null) it else it.copy(setup = next) }
        }
    }

    /** Saves the checked token, restarts hq so the relay starts, and goes on to pairing when no account is linked yet. */
    private fun saveToken(typed: String) {
        val setup = _state.value.setup ?: return
        val token = typed.trim()
        if (token.isEmpty() || setup.saving || _state.value.restarting) return
        _state.update { it.copy(setup = setup.copy(saving = true, error = null), restarting = true) }
        viewModelScope.launch {
            val failure = withContext(Dispatchers.IO) {
                val before = startStamp().second
                try { client.postJson("/api/chat-apps/${setup.platform}", JSONObject().put("token", token)) } catch (e: HqUnavailable) { return@withContext SettingsWords.chatAppFailure(e.status, null) }
                HubService.restartHq()
                if (waitForHq(before)) null else RESTART_TIMEOUT_LINE
            }
            if (failure != null) { _state.update { it.copy(restarting = false, setup = it.setup?.copy(saving = false, error = failure)) }; return@launch }
            refreshNow()
            val linked = _state.value.snap.chatApps.let { SettingsWords.appCard(it, setup.platform, SettingsWords.appName(setup.platform)).owner == "Linked" }
            _state.update { it.copy(restarting = false, setup = it.setup?.copy(saving = false, saved = true), line = SettingsWords.SAVED_RESTARTED) }
            if (!linked) pairApp(setup.platform, false, replacePage = true)
            else _state.update { if (it.stack.lastOrNull() == SettingsPage.SetupApp) it.copy(stack = it.stack.dropLast(1), setup = null) else it.copy(setup = null) }
        }
    }

    /** Asks HQ for a code to send to the bot; [replacePage] swaps the setup page for the pairing page instead of stacking it. */
    private fun pairApp(platform: String, replace: Boolean, replacePage: Boolean = false) {
        _state.update { it.copy(askReplace = null) }
        viewModelScope.launch {
            try {
                val r = client.postJson("/api/chat-apps/$platform/pair", JSONObject().put("replace", replace))
                val pair = PairState(platform, r.optString("code"), r.optString("command"), r.takeIf { o -> o.has("deep_link") && !o.isNull("deep_link") }?.optString("deep_link"), r.optLong("expires_at"))
                _state.update {
                    val top = it.stack.lastOrNull()
                    val swaps = top == SettingsPage.PairApp || replacePage && top == SettingsPage.SetupApp
                    it.copy(pair = pair, stack = (if (swaps) it.stack.dropLast(1) else it.stack) + SettingsPage.PairApp)
                }
            } catch (e: HqUnavailable) {
                _state.update { it.copy(line = SettingsWords.chatAppFailure(e.status, null)) }
            }
        }
    }

    /** While a code waits: reads the chat apps again, and leaves for Messaging apps once the account is linked. */
    private suspend fun pairPoll() {
        val pair = _state.value.pair ?: return
        if (pair.expiresAt <= java.time.Instant.now().epochSecond) return
        val body = try { client.json("/api/chat-apps") } catch (e: HqUnavailable) { return }
        val linked = SettingsWords.appCard(body, pair.platform, SettingsWords.appName(pair.platform)).owner == "Linked"
        _state.update { s ->
            val withBody = s.copy(snap = s.snap.copy(chatApps = body))
            if (linked && s.page == SettingsPage.PairApp) withBody.copy(stack = s.stack.dropLast(1), pair = null, line = "Linked.") else withBody
        }
    }

    /** Forgets the bot token and restarts hq so the relay stops. */
    private fun disconnectApp() {
        val platform = _state.value.askDisconnect ?: return
        if (_state.value.restarting) return
        _state.update { it.copy(askDisconnect = null, restarting = true) }
        viewModelScope.launch {
            val said = withContext(Dispatchers.IO) {
                val before = startStamp().second
                try { client.deleteAt("/api/chat-apps/$platform") } catch (e: HqUnavailable) { return@withContext if (e.status == 409) SettingsWords.chatAppFailure(409, null) else "Could not disconnect it. HQ is not answering." }
                HubService.restartHq()
                if (waitForHq(before)) SettingsWords.SAVED_RESTARTED else RESTART_TIMEOUT_LINE
            }
            _state.update { it.copy(restarting = false, line = said).withFacts() }
            refreshNow()
        }
    }

    /** One call to HQ from these pages: the tapped control is inert until it returns, a failure says so, and the page reads again either way. */
    private fun act(key: String, failure: String, block: suspend () -> Unit) {
        if (_state.value.busy != null) return
        _state.update { it.copy(busy = key) }
        viewModelScope.launch {
            val said = try { block(); null } catch (e: HqUnavailable) { if (e.status == HTTP_NOT_FOUND) "That is already gone." else failure }
            _state.update { it.copy(busy = null, line = said ?: it.line) }
            refreshNow()
        }
    }

    private fun newCode() = act("code", "Could not get a code. HQ is not answering.") {
        val o = client.postJson("/api/auth/device-code")
        val code = o.optString("code")
        if (code.isNotBlank()) _state.update { it.copy(deviceCode = DeviceCode(code, o.optLong("expires_at"))) }
    }

    /**
     * Signs out every other device. HQ also revokes this app's own sign-in, so a new one is sealed and hq restarted
     * with it at once; the app then waits for hq as Restart HQ does.
     */
    private fun signOutAll() {
        if (_state.value.restarting) return
        _state.update { it.copy(askSignOutAll = false, restarting = true, deviceCode = null) }
        viewModelScope.launch {
            val said = withContext(Dispatchers.IO) {
                val before = startStamp().second
                try { client.postJson("/api/auth/rotate") } catch (e: HqUnavailable) { return@withContext "Could not sign out. HQ is not answering." }
                HubService.recoverToken(ctx(), true)
                if (waitForHq(before)) { dropRotateDevice(); "Signed out everywhere." } else RESTART_TIMEOUT_LINE
            }
            _state.update { it.copy(restarting = false, line = said).withFacts() }
            refreshNow()
        }
    }

    /** Signing out everywhere leaves one new sign-in for "this device", which the app never uses: it would show as a stranger signed in, so it is removed. */
    private suspend fun dropRotateDevice() {
        try {
            client.json("/api/auth/devices").optJSONArray("devices")?.let { a ->
                for (i in 0 until a.length()) {
                    val d = a.optJSONObject(i) ?: continue
                    if (d.optString("label") == SettingsWords.ROTATE_LABEL && !d.optBoolean("current")) client.deleteAt("/api/auth/devices/${java.net.URLEncoder.encode(d.optString("id"), "UTF-8")}")
                }
            }
        } catch (e: HqUnavailable) {
            // Left in the list; the owner can sign it out there.
        }
    }

    private fun copyText(text: String) {
        val cm = ctx().getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
        cm.setPrimaryClip(android.content.ClipData.newPlainText("AgentHQ", text))
        _state.update { it.copy(line = "Copied.") }
    }

    private fun captureFacts() = CaptureFacts(CapturePrefs.shareOpens(ctx()), CapturePrefs.showTitles(ctx()), Outbox.of(ctx()).all())

    private val widgetKinds = listOf<Class<*>>(CaptureWidget::class.java, NeedsYouWidget::class.java, TasksWidget::class.java)

    /** Asks the launcher to put one of the three widgets on the home screen, which it confirms itself. */
    private fun addWidget(which: Int) {
        val ok = Widgets.pin(ctx(), widgetKinds[which])
        if (!ok) _state.update { it.copy(line = "Your launcher cannot add it from here. Press and hold the home screen and choose Widgets.") }
    }

    /** Tries one saved item again now, then reads what is left. */
    private fun sendNow(id: String) {
        Outbox.of(ctx()).retry(id)
        viewModelScope.launch(Dispatchers.IO) {
            OutboxSender.flushNow(ctx())
            _state.update { it.copy(capture = captureFacts(), phone = phoneRows()) }
        }
    }

    private fun notifFacts(): NotifFacts {
        val nm = ctx().getSystemService(NotificationManager::class.java)
        val channels = listOf(AlertPoster.NEEDS, AlertPoster.UPDATES, HUB_CHANNEL).mapNotNull { nm.getNotificationChannel(it) }
            .map { ChannelFacts(it.id, it.name.toString(), it.importance, it.sound != null) }
        return NotifFacts(nm.areNotificationsEnabled(), channels, AlertPoster.answersAllowed(ctx()), buttonsSupported = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S)
    }

    /** Opens the Android page that changes a setting the owner was told about; a page this phone does not have is tried past, never a crash. */
    private fun fix(what: PhoneFix) {
        keepCache = null
        startFirst(PhoneIntents.tries(ctx(), what, _state.value.keep.backgroundRestricted))
    }

    private fun openChannel(id: String) {
        val c = ctx()
        startFirst(listOf(Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, c.packageName).putExtra(Settings.EXTRA_CHANNEL_ID, id)))
    }

    private fun startFirst(tries: List<Intent>) {
        if (!PhoneIntents.startFirst(ctx(), tries)) _state.update { it.copy(line = "Android could not open that screen.") }
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
            val problem = withContext(Dispatchers.IO) { switchAndWait(toActive) }
            _state.update { it.copy(restarting = false, mode = modeOf(), phone = phoneRows(), line = problem).withFacts() }
            refreshNow()
        }
    }

    /** Null when hq is back running the chosen mode; otherwise the line that says what went wrong. */
    private suspend fun switchAndWait(toActive: Boolean): String? {
        val before = startStamp().second
        try { HubService.switchMode(ctx(), toActive) } catch (e: java.io.IOException) { return SAVE_FAILED_LINE }
        if (!waitForHq(before)) return RESTART_TIMEOUT_LINE
        if (HubService.childActive() == toActive) return null
        // The switch landed while hq was starting with the old mode: restart once more so it reads the file.
        val again = startStamp().second
        HubService.restartHq()
        return if (waitForHq(again)) null else RESTART_TIMEOUT_LINE
    }

    /** Stops hq and waits for it to answer again, with the mode left as it is. */
    private fun confirmRestart() {
        if (_state.value.restarting) return
        _state.update { it.copy(askRestart = false, restarting = true) }
        viewModelScope.launch {
            val said = withContext(Dispatchers.IO) {
                val before = startStamp().second
                HubService.restartHq()
                if (waitForHq(before)) "HQ restarted." else RESTART_TIMEOUT_LINE
            }
            _state.update { it.copy(restarting = false, line = said).withFacts() }
            refreshNow()
        }
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
        val cm = ctx().getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
        cm.setPrimaryClip(android.content.ClipData.newPlainText("HQ diagnostics", SettingsWords.diagnostics(s.build, s.mode == "active", s.snap.health, HubService.PORT)))
        _state.update { it.copy(line = "Copied.") }
    }

    private companion object {
        const val MODE_MAX = 64L
        const val EXIT_SCAN = 10
        const val HUB_CHANNEL = "hub"
        const val SAMSUNG_BATTERY_PACKAGE = "com.samsung.android.lool"
        const val SAMSUNG_BATTERY_PAGE = "com.samsung.android.sm.battery.ui.BatteryActivity"
        const val HTTP_NOT_FOUND = 404
        const val SAVE_FAILED_LINE = "Could not save the setting."
        const val KEEP_CACHE_MS = 5_000L
        const val RESTART_TIMEOUT_LINE = "HQ did not come back within two minutes."
    }
}
