package example.hq.spike.ui.settings

import example.hq.spike.ui.firstrun.FirstRunWords
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
import example.hq.spike.R
import example.hq.spike.ui.shell.Undo
import example.hq.spike.ui.shell.Words
import example.hq.spike.HubService
import example.hq.spike.ServerHq
import example.hq.spike.capture.CapturePrefs
import example.hq.spike.capture.CaptureWidget
import example.hq.spike.capture.NeedsYouWidget
import example.hq.spike.capture.Outbox
import example.hq.spike.capture.OutboxSender
import example.hq.spike.capture.TasksWidget
import example.hq.spike.capture.Widgets
import example.hq.spike.data.HqClient
import example.hq.spike.data.HqServices
import example.hq.spike.safe.NotesCopy
import example.hq.spike.data.HqUnavailable
import example.hq.spike.update.ChannelManifest
import example.hq.spike.update.UpdateChannel
import example.hq.spike.update.UpdateState
import example.hq.spike.update.Updater
import java.io.File
import java.time.Instant
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
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
    val trustedSites: List<String> = emptyList(),
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
    /** What is being asked or done on the Other devices page. */
    val busy: String? = null,
    val askSignOutAll: Boolean = false,
    /** The device the owner is being asked to sign out, and why the last try failed. */
    val askSignOut: SettingsWords.Device? = null,
    val signOutError: String? = null,
    val askServerDisconnect: Boolean = false,
    /** Captures hidden while their Undo snackbar shows; the file is deleted when it closes. */
    val hiddenCaptures: Set<String> = emptySet(),
    val undoLine: String? = null,
    val deviceCode: DeviceCode? = null,
    val setup: SetupState? = null,
    val pair: PairState? = null,
    val askDisconnect: String? = null,
    val askReplace: String? = null,
    /** Why signing in to a server HQ did not work, shown under its fields. */
    val serverError: String? = null,
    /** The in-app updater: what it last found, and whether the owner follows every build of main instead of promoted ones. */
    val update: UpdateState = UpdateState.Idle,
    val updateMain: Boolean = false,
    val updateDot: Boolean = false,
    val updateHidden: Boolean = false,
    val updateFacts: Updater.Facts = Updater.Facts(),
    /** Every large download's state, by id, and the Wi-Fi and unfinished-download figures the Mobile data and Storage groups show. */
    val downloads: Map<String, example.hq.spike.download.DlState> = emptyMap(),
    val wifiFree: Int = 0,
    val wifiPaid: Int = 0,
    val askAnyNetwork: Boolean = false,
    val unfinishedBytes: Long = 0,
    val dlUndo: DlUndo? = null,
    /** Bytes the speech model takes on this phone, or -1 when it is not downloaded; and the delete question. */
    val speechBytes: Long = -1,
    val askDeleteSpeech: Boolean = false,
    /** Keys on this phone: the open "Remove this key?" question, what the section last said, a bump that makes it read the keys again, and a name to fill in for Replace. */
    val keyRemove: KeyRemove? = null,
    val keysLine: String? = null,
    val keysLineBad: Boolean = false,
    val keysVersion: Int = 0,
    val keyReplace: String? = null,
    /** When HQ last answered, and whether what is shown now includes values kept from that earlier answer. */
    val savedAt: Instant? = null,
    val stale: Boolean = false,
    /** Save a copy now: idle, running, done or failed. */
    val copy: BackupCopy.State = BackupCopy.State.Idle,
    val silences: Silences? = null,
    val silenceBusy: Boolean = false,
) {
    val page: SettingsPage get() = stack.lastOrNull() ?: SettingsPage.Home

    /** A key is being checked or a bot token saved: Back waits, so it does not close the page that work belongs to. */
    fun busyBehind(): Boolean = keySheet?.checking == true || setup?.saving == true
}

/** What the Undo bar after a Mobile data or Storage change puts back. */
enum class DlUndo { Forget, Remove }

class SettingsActions(
    val onOpen: () -> Unit = {},
    val onBack: () -> Unit = {},
    val onPage: (SettingsPage) -> Unit = {},
    val onTab: (SettingsWords.SafetyTab) -> Unit = {},
    val onForgetSite: (String) -> Unit = {},
    val onPhoneRow: (String) -> Unit = {},
    /** Set by the shell: opens the GitHub connection over the place that is showing. */
    var onCodeFromPhone: () -> Unit = {},
    /** Set by the shell: opens Tasks or Helpers, by the row's key. */
    var onWork: (String) -> Unit = {},
    val onCopyDiagnostics: () -> Unit = {},
    val onDismissLine: () -> Unit = {},
    val onMode: (Boolean) -> Unit = {},
    val onModeCancel: () -> Unit = {},
    val onModeConfirm: () -> Unit = {},
    val onRestart: (Boolean) -> Unit = {},
    val onRestartConfirm: () -> Unit = {},
    val onFix: (PhoneFix) -> Unit = {},
    val onChannel: (String) -> Unit = {},
    val onSilenceTopic: (String, Boolean) -> Unit = { _, _ -> },
    val onPause: (Int) -> Unit = {},
    val onResume: () -> Unit = {},
    val onDetails: (Boolean) -> Unit = {},
    val onUpgraderNoteSeen: () -> Unit = {},
    val onServerConnect: (String, String) -> Unit = { _, _ -> },
    val onServerDisconnect: () -> Unit = {},
    val onShareOpens: (String) -> Unit = {},
    val onSetup: (String) -> Unit = {},
    val onCheckToken: (String) -> Unit = {},
    val onSaveToken: (String) -> Unit = {},
    val onPair: (String, Boolean) -> Unit = { _, _ -> },
    val onPairPoll: suspend () -> Unit = {},
    val onAskDisconnect: (String?) -> Unit = {},
    val onDisconnect: () -> Unit = {},
    val onAskReplace: (String?) -> Unit = {},
    val onNewCode: () -> Unit = {},
    val onAskSignOut: (SettingsWords.Device?) -> Unit = {},
    val onSignOutConfirm: () -> Unit = {},
    val onAskServerDisconnect: (Boolean) -> Unit = {},
    val onUndoDiscard: () -> Unit = {},
    /** The Undo line closed by itself: a discarded capture is deleted for good. */
    val onUndoExpire: () -> Unit = {},
    val onDlUndoExpire: () -> Unit = {},
    val onSignOutAll: (Boolean) -> Unit = {},
    val onSignOutAllConfirm: () -> Unit = {},
    val onCopy: (String) -> Unit = {},
    val onKeyOpen: (KeySheet) -> Unit = {},
    val onAskRemoveKey: (String?) -> Unit = {},
    val onRemoveKey: () -> Unit = {},
    val onReplaceKey: () -> Unit = {},
    val onKeyReplaceHandled: () -> Unit = {},
    val onKeyClose: () -> Unit = {},
    val onKeySave: (String) -> Unit = {},
    val onShowTitles: (Boolean) -> Unit = {},
    val onSmartLinks: (Boolean) -> Unit = {},
    val onAddWidget: (Int) -> Unit = {},
    val onSendNow: (String) -> Unit = {},
    val onDiscard: (String) -> Unit = {},
    /** One read of everything Settings shows; the page calls it every few seconds while it is showing and the app is in front. */
    val onRefresh: suspend () -> Unit = {},
    val onCheckUpdate: () -> Unit = {},
    val onInstallUpdate: (ChannelManifest) -> Unit = {},
    /** The busy dialog: wait until HQ is free, stop what runs and install, or leave it offered. */
    val onWaitThenInstall: (ChannelManifest) -> Unit = {},
    val onStopAndInstall: (ChannelManifest) -> Unit = {},
    val onBusyDismiss: (ChannelManifest) -> Unit = {},
    val onUpdateMain: (Boolean) -> Unit = {},
    /** Save a copy now: the picked document, cancelling a copy under way, closing its Done or Failed card, and Got it. */
    val onSaveCopy: (android.net.Uri) -> Unit = {},
    val onSaveCopyCancel: () -> Unit = {},
    val onSaveCopyClear: () -> Unit = {},
    val onGotIt: () -> Unit = {},
    val onUpdatePageOpened: () -> Unit = {},
    val onUpdateNotNow: () -> Unit = {},
    val onAskAnyNetwork: (Boolean) -> Unit = {},
    val onForgetWifi: () -> Unit = {},
    val onRemoveUnfinished: () -> Unit = {},
    val onDlUndo: () -> Unit = {},
    val onAskDeleteSpeech: (Boolean) -> Unit = {},
    val onDeleteSpeech: () -> Unit = {},
)

class SettingsViewModel(app: Application) : AndroidViewModel(app) {
    private val client = HqServices.get(app).client
    private val _state = MutableStateFlow(SettingsUi(build = buildText(app)))
    val state: StateFlow<SettingsUi> = _state
    private var frozen = false

    val actions = SettingsActions(
        onOpen = ::open,
        onBack = { _state.update { if (it.busyBehind()) it else if (it.stack.isEmpty()) it.copy(open = false) else it.copy(stack = it.stack.dropLast(1)) } },
        onPage = { p ->
            _state.update { it.copy(stack = it.stack + p) }
            if (p == SettingsPage.Notifications) loadSilences()
        },
        onTab = { t -> _state.update { it.copy(safetyTab = t) } },
        onForgetSite = ::forgetSite,
        onPhoneRow = ::phoneRow,
        onSilenceTopic = ::silenceTopic,
        onPause = ::pause,
        onResume = ::resume,
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
        onNewCode = ::newCode,
        onAskSignOut = { d -> _state.update { it.copy(askSignOut = d, signOutError = null) } },
        onSignOutConfirm = ::signOutDevice,
        onAskServerDisconnect = { ask -> _state.update { it.copy(askServerDisconnect = ask) } },
        onUndoDiscard = ::undoDiscard, onUndoExpire = ::flushDiscard,
        onDlUndoExpire = { _state.update { it.copy(dlUndo = null) } },
        onSignOutAll = { ask -> _state.update { if (it.restarting) it else it.copy(askSignOutAll = ask) } },
        onSignOutAllConfirm = ::signOutAll,
        onCopy = { text -> copyText(text) },
        onKeyOpen = { k -> _state.update { it.copy(keySheet = k) } },
        onKeyClose = { _state.update { if (it.keySheet?.checking == true) it else it.copy(keySheet = null) } },
        onKeySave = ::saveKey,
        onShareOpens = { v -> CapturePrefs.setShareOpens(ctx(), v); _state.update { it.copy(capture = it.capture.copy(shareOpens = v)) } },
        onShowTitles = { on -> CapturePrefs.setShowTitles(ctx(), on); Widgets.renderAll(ctx()); _state.update { it.copy(capture = it.capture.copy(showTitles = on)) } },
        onSmartLinks = { on -> CapturePrefs.setSmartLinks(ctx(), on); _state.update { it.copy(capture = it.capture.copy(smartLinks = on)) } },
        onAddWidget = ::addWidget, onSendNow = ::sendNow, onDiscard = ::discard,
        onDetails = { on -> AlertPoster.setShowDetails(ctx(), on); _state.update { it.copy(notif = it.notif.copy(showDetails = on)) } },
        onUpgraderNoteSeen = { AlertPoster.clearUpgraderNote(ctx()) },
        onServerConnect = ::connectServer,
        onServerDisconnect = ::disconnectServer,
        onRefresh = ::refreshNow,
        onCheckUpdate = { viewModelScope.launch(Dispatchers.IO) { Updater.check(ctx()) } },
        onInstallUpdate = { m -> viewModelScope.launch(Dispatchers.IO) { Updater.install(ctx(), m) } },
        onWaitThenInstall = { m -> viewModelScope.launch(Dispatchers.IO) { Updater.waitThenInstall(ctx(), m) } },
        onStopAndInstall = { m -> viewModelScope.launch(Dispatchers.IO) { Updater.install(ctx(), m, stopAndInstall = true) } },
        onBusyDismiss = { m -> Updater.dismissBusy(m) },
        onSaveCopy = { uri -> BackupCopy.start(ctx(), uri) },
        onSaveCopyCancel = BackupCopy::cancel,
        onSaveCopyClear = BackupCopy::clear,
        onGotIt = { java.io.File(ctx().filesDir, GOT_IT).writeText("1"); _state.update { it.withFacts() } },
        onUpdatePageOpened = { viewModelScope.launch(Dispatchers.IO) { Updater.pageOpened(ctx()); _state.update { it.withDownloadFacts() } } },
        onUpdateNotNow = { viewModelScope.launch(Dispatchers.IO) { Updater.notNow(ctx()) } },
        onAskAnyNetwork = { on -> example.hq.spike.download.Downloads.setAskAnyNetwork(ctx(), on); _state.update { it.copy(askAnyNetwork = on) } },
        onForgetWifi = {
            forgotten = example.hq.spike.download.PaidWifi(example.hq.spike.download.PrefsAnswers(ctx())).forget()
            _state.update { it.withDownloadFacts().copy(dlUndo = DlUndo.Forget) }
        },
        onRemoveUnfinished = {
            viewModelScope.launch(Dispatchers.IO) {
                val any = example.hq.spike.download.Downloads.removeUnfinished(ctx())
                _state.update { it.withDownloadFacts().copy(dlUndo = if (any) DlUndo.Remove else null) }
            }
        },
        onDlUndo = {
            viewModelScope.launch(Dispatchers.IO) {
                when (_state.value.dlUndo) {
                    DlUndo.Forget -> example.hq.spike.download.PaidWifi(example.hq.spike.download.PrefsAnswers(ctx())).restore(forgotten)
                    DlUndo.Remove -> example.hq.spike.download.Downloads.undoRemove(ctx())
                    null -> {}
                }
                _state.update { it.withDownloadFacts().copy(dlUndo = null) }
            }
        },
        onAskRemoveKey = { name -> if (name == null) _state.update(KeyRemoval::cancel) else askRemoveKey(name) },
        onRemoveKey = ::removeKey,
        onReplaceKey = { _state.update(KeyRemoval::replace) },
        onKeyReplaceHandled = { _state.update(KeyRemoval::replaceHandled) },
        onAskDeleteSpeech = { ask -> _state.update { it.copy(askDeleteSpeech = ask) } },
        onDeleteSpeech = {
            viewModelScope.launch(Dispatchers.IO) {
                example.hq.spike.voice.VoiceModels(ctx()).delete()
                _state.update { it.withDownloadFacts().copy(askDeleteSpeech = false, line = ctx().getString(R.string.speech_deleted), phone = phoneRows()) }
            }
        },
        onUpdateMain = { main ->
            Updater.setChannel(ctx(), if (main) UpdateChannel.Main else UpdateChannel.Stable)
            _state.update { it.copy(updateMain = main) }
        },
    )

    init {
        _state.update { it.copy(updateMain = Updater.channel(app) == UpdateChannel.Main) }
        viewModelScope.launch { Updater.state.collect { s -> _state.update { it.copy(update = s).withDownloadFacts() } } }
        viewModelScope.launch { Updater.dot.collect { d -> _state.update { it.copy(updateDot = d) } } }
        viewModelScope.launch { Updater.hidden.collect { h -> _state.update { it.copy(updateHidden = h) } } }
        viewModelScope.launch { Updater.line.collect { l -> if (l != null) { _state.update { it.copy(line = ctx().getString(l.id, l.arg)) }; Updater.clearLine() } } }
        viewModelScope.launch { example.hq.spike.download.Downloads.states.collect { d -> _state.update { it.copy(downloads = d) } } }
        viewModelScope.launch { BackupCopy.state.collect { c -> _state.update { it.copy(copy = c).withFacts() } } }
    }

    private var forgotten: Map<String, Boolean> = emptyMap()

    private fun SettingsUi.withDownloadFacts(): SettingsUi {
        val c = ctx()
        val wifi = example.hq.spike.download.PaidWifi(example.hq.spike.download.PrefsAnswers(c))
        return copy(
            updateFacts = Updater.facts(c), wifiFree = wifi.free(), wifiPaid = wifi.paid(),
            askAnyNetwork = example.hq.spike.download.Downloads.askAnyNetwork(c), unfinishedBytes = example.hq.spike.download.Downloads.unfinishedBytes(c),
            speechBytes = example.hq.spike.voice.VoiceModels(c).let { if (it.ready()) it.onPhoneBytes() else -1L },
        )
    }

    /** Closes Settings and every page inside it. */
    fun close() = _state.update { it.copy(open = false, stack = emptyList(), askMode = null, askRestart = false, keySheet = null, askSignOutAll = false, deviceCode = null, setup = null, pair = null, askDisconnect = null, askReplace = null) }

    fun freeze(fixed: SettingsUi) {
        frozen = true
        _state.value = fixed
    }

    /** Opens Settings on its home page. What is on it comes from [refreshNow], which the page runs while it is showing. */
    private fun open() {
        _state.update { it.copy(open = true, stack = emptyList(), line = null, askMode = null, askRestart = false, phone = if (frozen) it.phone else phoneRows(), mode = if (frozen) it.mode else modeOf(), now = Instant.now()).withFacts().withDownloadFacts() }
    }

    /**
     * Reads every route at once. The seven local ones are shown as soon as they answer; the three that ask a provider or add up a month (which can
     * take many seconds) are added when they arrive. One route that fails shows as such on its own row only.
     */
    suspend fun refreshNow() {
        if (frozen) return
        val local = listOf("settings" to "/api/settings", "chatApps" to "/api/chat-apps", "devices" to "/api/auth/devices", "budgets" to "/api/budgets", "health" to "/health", "vault" to "/api/vault-status", TRUSTED_SITES to "/api/fetch-sites")
        val remote = listOf("providers" to "/api/usage/providers", "copilot" to "/api/copilot-usage", "forecast" to "/api/usage/forecast", "github" to "/api/github")
        coroutineScope {
            val slow = remote.map { (key, path) -> async { key to get(path) } }
            val fast = local.map { (key, path) -> async { key to get(path) } }.awaitAll().toMap()
            if (frozen) return@coroutineScope
            val server = async { serverReport() }
            val before = _state.value.snap
            publish(fast, mapOf("providers" to Got(before.providers, before.providers != null), "copilot" to Got(before.copilot, before.copilot != null), "forecast" to Got(before.forecast, before.forecast != null), "github" to Got(before.github, before.github != null)))
            val late = slow.awaitAll().toMap()
            serverBackup = server.await()
            if (frozen) return@coroutineScope
            publish(fast, late)
        }
    }

    private fun loadSilences() = changeSilences(quiet = true) { client.json(SILENCES_PATH) }

    private fun silenceTopic(topic: String, silence: Boolean) = changeSilences {
        if (silence) client.postJson(SILENCES_PATH, JSONObject().put("topic", topic)) else client.deleteJson("$SILENCES_PATH/$topic")
    }

    private fun pause(minutes: Int) = changeSilences { client.postJson(SILENCES_PATH, JSONObject().put("topic", SILENCE_ALL).put("minutes", minutes)) }

    private fun resume() = changeSilences { client.deleteJson("$SILENCES_PATH/$SILENCE_ALL") }

    /** Every silence call answers with the whole list, which replaces what is shown. A first read that fails ([quiet]) leaves the section out: an older HQ has no such route. */
    private fun changeSilences(quiet: Boolean = false, call: suspend () -> JSONObject) {
        if (frozen) return
        _state.update { it.copy(silenceBusy = true) }
        viewModelScope.launch {
            val now = try { SettingsWords.silences(call()) } catch (e: HqUnavailable) { null }
            _state.update { it.copy(silenceBusy = false, silences = now ?: it.silences, line = if (now == null && !quiet) ctx().getString(R.string.inbox_fail_no_reply) else it.line) }
        }
    }

    private fun askRemoveKey(name: String) {
        viewModelScope.launch {
            val only = withContext(Dispatchers.IO) { example.hq.spike.KeyVault.isOnlyModelKey(example.hq.spike.KeyVault.names(ctx()), name) }
            _state.update { KeyRemoval.ask(it, name, only) }
        }
    }

    private fun removeKey() {
        val q = _state.value.keyRemove ?: return
        if (q.busy) return
        _state.update(KeyRemoval::begin)
        viewModelScope.launch {
            val ok = withContext(Dispatchers.IO) { example.hq.spike.KeyVault.remove(ctx(), q.name).also { if (it) HubService.restartHq() } }
            val line = if (ok) ctx().getString(if (q.only) R.string.remove_only_key_done else R.string.remove_key_done) else FirstRunWords.KEYS_NOT_REMOVED
            _state.update { KeyRemoval.finished(it, line, ok) }
        }
    }

    private fun forgetSite(site: String) {
        viewModelScope.launch {
            val gone = try { client.deleteAt("/api/fetch-sites/${java.net.URLEncoder.encode(site, "UTF-8")}"); true } catch (e: HqUnavailable) { false }
            if (gone) _state.update { it.copy(trustedSites = it.trustedSites - site) }
        }
    }

    private fun publish(fast: Map<String, Got>, late: Map<String, Got>) {
        val got = fast + late
        val before = _state.value.snap
        val kept = mapOf(
            "settings" to before.settings, "chatApps" to before.chatApps, "devices" to before.devices, "budgets" to before.budgets, "health" to before.health,
            "providers" to before.providers, "copilot" to before.copilot, "forecast" to before.forecast, "vault" to before.vault, "github" to before.github,
        )
        // A route HQ did not answer at all keeps its last values; an error answer is still shown as one.
        val silent = got.filterValues { it.body == null && !it.answered }.keys.filter { kept[it] != null }
        val answeredAny = got.values.any { it.answered }
        fun body(key: String) = got[key]?.body ?: kept[key].takeIf { key in silent }
        val snap = SettingsSnapshot(
            body("settings"), body("chatApps"), body("devices"), body("budgets"), body("health"),
            unavailable = got.filterValues { it.body == null && it.answered }.keys,
            providers = body("providers"), copilot = body("copilot"), forecast = body("forecast"), vault = body("vault"), github = body("github"),
        )
        val now = Instant.now()
        val sites = got[TRUSTED_SITES]?.body?.optJSONArray("sites")?.let { a -> (0 until a.length()).map(a::getString) }
        if (sites != null) _state.update { it.copy(trustedSites = sites) }
        _state.update { it.copy(snap = snap, loaded = true, phone = phoneRows(), mode = modeOf(), now = now, savedAt = if (answeredAny) now else it.savedAt, stale = silent.isNotEmpty() && !answeredAny).withFacts() }
    }

    /** What one route gave back: its JSON, or none; and whether HQ answered at all (an error answer is not the same as no answer). */
    private class Got(val body: JSONObject?, val answered: Boolean)

    private suspend fun get(path: String): Got = try { Got(client.json(path), true) } catch (e: HqUnavailable) { Got(null, e.message?.startsWith("HTTP ") == true) }

    private fun ctx(): Application = getApplication()
    private fun words() = Words.of(ctx())

    /** What a server HQ says about its backups; null when this phone is not signed in to one, or the server does not say. */
    @Volatile private var serverBackup: HqClient.ServerBackup? = null

    private suspend fun serverReport(): HqClient.ServerBackup? {
        val server = ServerHq.current(ctx()) ?: return null
        if (modeOf() == "active") return null
        return HqClient({ server.token }, server.address).backupStatus()
    }

    /** "hub · build 1": the version name and the build number, as the board writes them. */
    private fun buildText(app: Application): String {
        val info = app.packageManager.getPackageInfo(app.packageName, 0)
        @Suppress("DEPRECATION")
        val number = if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.P) info.longVersionCode else info.versionCode.toLong()
        return app.getString(R.string.settings_build_line, info.versionName.orEmpty().ifBlank { "hub" }, number)
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
        val keepToFix = keepFixCount()
        // The page list is SettingsWords.PHONE_PAGES: a row for a page not in it is dropped, so Home and the placement test read one list.
        return SettingsWords.inPhoneOrder(listOfNotNull(
            HomeRow(SettingsPage.Background, "background", c.getString(R.string.about_background), SettingsWords.backgroundHome(Words.of(c), active, keepToFix), warn = keepToFix > 0),
            HomeRow(SettingsPage.Notifications, "notifications", c.getString(R.string.row_notifications), if (!allowed) c.getString(R.string.assist_mic_off) else homeNotifLine(listOf(R.string.notify_home_needs_on to on(AlertPoster.NEEDS), R.string.notify_home_updates_on to on(AlertPoster.UPDATES))), warn = SettingsWords.notificationsWarn(allowed, on(AlertPoster.NEEDS), on(AlertPoster.UPDATES))),
            assistantRow(c),
            calendarRow(c),
            contactsRow(c),
            HomeRow(SettingsPage.CaptureWidgets, "capture", c.getString(R.string.settings_page_capture), PhoneWords.captureSummary(Words.of(c), captureFacts())),
        ))
    }

    /** How many Keep running checks need a fix; the line on Background work and the Home warning card both read it. */
    private fun keepFixCount(): Int = PhoneWords.keepRunning(Words.of(ctx()), keepFacts(), Instant.now()).checked.count { !it.ok }

    private fun calendarRow(c: Context): HomeRow {
        val allowed = example.hq.spike.calendar.AndroidCalendarStore.hasPermission(c)
        val chosen = example.hq.spike.calendar.CalendarPrefs(c).get()
        val status = c.getString(if (!allowed) R.string.cal_status_no_access else if (chosen == null) R.string.cal_status_none else R.string.cal_chosen)
        return HomeRow(SettingsPage.Calendar, "calendar", c.getString(R.string.cal_row_title), status, warn = !allowed || chosen == null)
    }

    private fun contactsRow(c: Context): HomeRow {
        val allowed = example.hq.spike.contacts.AndroidContactsReader.hasPermission(c)
        return HomeRow(SettingsPage.Contacts, "contacts", c.getString(R.string.contacts_row_title), c.getString(if (allowed) R.string.contacts_status_allowed else R.string.cal_status_no_access), warn = !allowed)
    }

    private fun assistantRow(c: Context): HomeRow {
        val role = example.hq.spike.voice.AssistantRole
        val available = role.available(c)
        val held = role.held(c)
        val mic = role.micState(c)
        val status = AssistantWords.status(Words.of(c), available, held, mic, example.hq.spike.voice.VoiceModels(c).ready())
        return HomeRow(SettingsPage.Assistant, "assistant", c.getString(R.string.assist_row_title), status, warn = AssistantWords.warn(available, held, mic))
    }

    /** "Needs you on · Updates on · App updates on": a part that is off reads with the word off in place of on. */
    private fun homeNotifLine(parts: List<Pair<Int, Boolean>>): String = parts.joinToString(" \u00b7 ") { (res, on) ->
        val s = ctx().getString(res)
        if (on) s else s.removeSuffix(" on") + " off"
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
        var readable = true
        // A missing file is "absent"; a file that is there but cannot be read is "could not check".
        fun read(f: java.io.File): String? = try { f.readText() } catch (e: java.io.FileNotFoundException) { if (f.exists()) readable = false; null } catch (e: java.io.IOException) { readable = false; null }
        val config = read(java.io.File(files, "hqhome/config.yaml"))?.let { BackupWords.parseConfig(it) }
        val named = config?.dir?.let { java.io.File(it) }
        val inside = named?.let { runCatching { it.canonicalPath.startsWith(files.canonicalPath + "/") }.getOrDefault(false) } ?: true
        val dir = if (inside && named != null) named else java.io.File(files, "backups")
        val status = read(java.io.File(dir, "backup-status.json"))?.let { BackupWords.parseStatus(it) }
        val newest = dir.listFiles { f -> f.name.startsWith("hq-backup-") && f.name.endsWith(".hqbak") }?.maxByOrNull { it.lastModified() }
        val seen = java.io.File(files, "backup-first-seen")
        val enabled = config?.enabled == true
        val firstSeen = if (enabled) (seen.takeIf { it.exists() }?.readText()?.toLongOrNull() ?: System.currentTimeMillis().also { runCatching { seen.writeText(it.toString()) } }) else null
        val offPhone = NotesCopy.lastCopyOffPhone(files)
        val mode = modeOf()
        val onServer = ServerHq.current(ctx()) != null && mode != "active"
        return BackupFacts(
            config, status, newest?.lastModified(), newest?.name, readable, !inside && enabled, firstSeen, offPhone,
            server = onServer, serverName = if (onServer) ServerHq.address(ctx())?.substringAfter("://")?.substringBefore('/') else null,
            serverReport = serverBackup.takeIf { onServer }, paused = mode != "active", saveCopyOn = Updater.saveCopyAllowed(ctx()),
            gotItDismissed = java.io.File(files, GOT_IT).exists(),
        )
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
                _state.update { it.copy(keySheet = null, line = SettingsWords.keySaved(words(), sheet.name)) }
                refreshNow()
            } catch (e: HqUnavailable) {
                _state.update { it.copy(keySheet = it.keySheet?.copy(checking = false, error = SettingsWords.keyFailure(words(), e.status))) }
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
                    setup.copy(checking = false, found = name?.let { n -> if (setup.platform == "telegram") "@$n" else n } ?: ctx().getString(R.string.settings_your_bot), appId = r.optString("application_id").takeIf { a -> a.isNotBlank() }, intentOff = setup.platform == "discord" && r.has("message_content") && !r.optBoolean("message_content"))
                } else setup.copy(checking = false, error = r.optString("error").takeIf { e -> e.isNotBlank() } ?: SettingsWords.tokenRefused(words()))
            } catch (e: HqUnavailable) {
                setup.copy(checking = false, error = SettingsWords.chatAppFailure(words(), e.status, null))
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
                try { client.postJson("/api/chat-apps/${setup.platform}", JSONObject().put("token", token)) } catch (e: HqUnavailable) { return@withContext SettingsWords.chatAppFailure(words(), e.status, null) }
                HubService.restartHq()
                if (waitForHq(before)) null else restartTimeoutLine()
            }
            if (failure != null) { _state.update { it.copy(restarting = false, setup = it.setup?.copy(saving = false, error = failure)) }; return@launch }
            refreshNow()
            val linked = _state.value.snap.chatApps.let { SettingsWords.appCard(words(), it, setup.platform, SettingsWords.appName(setup.platform)).linked }
            _state.update { it.copy(restarting = false, setup = it.setup?.copy(saving = false, saved = true), line = SettingsWords.savedRestarted(words())) }
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
                _state.update { it.copy(line = SettingsWords.chatAppFailure(words(), e.status, null)) }
            }
        }
    }

    /** While a code waits: reads the chat apps again, and leaves for Messaging apps once the account is linked. */
    private suspend fun pairPoll() {
        val pair = _state.value.pair ?: return
        if (pair.expiresAt <= java.time.Instant.now().epochSecond) return
        val body = try { client.json("/api/chat-apps") } catch (e: HqUnavailable) { return }
        val linked = SettingsWords.appCard(words(), body, pair.platform, SettingsWords.appName(pair.platform)).linked
        _state.update { s ->
            val withBody = s.copy(snap = s.snap.copy(chatApps = body))
            if (linked && s.page == SettingsPage.PairApp) withBody.copy(stack = s.stack.dropLast(1), pair = null, line = ctx().getString(R.string.settings_linked_line)) else withBody
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
                try { client.deleteAt("/api/chat-apps/$platform") } catch (e: HqUnavailable) { return@withContext if (e.status == 409) SettingsWords.chatAppFailure(words(), 409, null) else ctx().getString(R.string.fail_disconnect_app_no_reply, SettingsWords.appName(platform)) }
                HubService.restartHq()
                if (waitForHq(before)) ctx().getString(R.string.disconnect_app_done, SettingsWords.appName(platform)) else restartTimeoutLine()
            }
            _state.update { it.copy(restarting = false, line = said).withFacts() }
            refreshNow()
        }
    }

    /** Signs one device out after the question; the dialog stays on a failure, with the reason in amber. */
    private fun signOutDevice() {
        val d = _state.value.askSignOut ?: return
        if (_state.value.busy != null) return
        _state.update { it.copy(busy = "out:${d.id}", signOutError = null) }
        viewModelScope.launch {
            val failed = try { client.deleteAt("/api/auth/devices/${java.net.URLEncoder.encode(d.id, "UTF-8")}"); false } catch (e: HqUnavailable) { e.status != HTTP_NOT_FOUND }
            _state.update {
                if (failed) it.copy(busy = null, signOutError = ctx().getString(R.string.fail_sign_out_no_reply, d.label))
                else it.copy(busy = null, askSignOut = null, line = ctx().getString(R.string.sign_out_device_done, d.label))
            }
            refreshNow()
        }
    }

    private var discarding: Job? = null

    /** Hides a waiting capture now and deletes it when the Undo snackbar closes. */
    private fun discard(id: String) {
        flushDiscard()
        val kind = Outbox.of(ctx()).all().firstOrNull { it.id == id }
        val text = ctx().getString(if (kind?.isNote == true) R.string.undo_note_discarded else R.string.undo_task_discarded)
        _state.update { it.copy(hiddenCaptures = it.hiddenCaptures + id, undoLine = text) }
    }

    private fun flushDiscard() {
        discarding?.cancel()
        discarding = null
        val hidden = _state.value.hiddenCaptures
        if (hidden.isEmpty()) return
        hidden.forEach { Outbox.of(ctx()).remove(it) }
        _state.update { it.copy(hiddenCaptures = emptySet(), undoLine = null, capture = captureFacts(), phone = phoneRows()) }
    }

    private fun undoDiscard() {
        discarding?.cancel()
        discarding = null
        _state.update { it.copy(hiddenCaptures = emptySet(), undoLine = null) }
    }

    override fun onCleared() {
        flushDiscard()
        super.onCleared()
    }

    /** One call to HQ from these pages: the tapped control is inert until it returns, a failure says so, and the page reads again either way. */
    private fun act(key: String, failure: String, block: suspend () -> Unit) {
        if (_state.value.busy != null) return
        _state.update { it.copy(busy = key) }
        viewModelScope.launch {
            val said = try { block(); null } catch (e: HqUnavailable) { if (e.status == HTTP_NOT_FOUND) ctx().getString(R.string.settings_already_gone) else failure }
            _state.update { it.copy(busy = null, line = said ?: it.line) }
            refreshNow()
        }
    }

    private fun newCode() = act("code", ctx().getString(R.string.settings_code_failed)) {
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
                try { client.postJson("/api/auth/rotate") } catch (e: HqUnavailable) { return@withContext ctx().getString(R.string.fail_sign_out_all_no_reply) }
                HubService.recoverToken(ctx(), true)
                if (waitForHq(before)) { dropRotateDevice(); ctx().getString(R.string.sign_out_all_done) } else restartTimeoutLine()
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
        _state.update { it.copy(line = ctx().getString(R.string.copy_details_copied)) }
    }

    private fun captureFacts() = CaptureFacts(CapturePrefs.shareOpens(ctx()), CapturePrefs.showTitles(ctx()), Outbox.of(ctx()).all(), CapturePrefs.smartLinks(ctx()))

    private val widgetKinds = listOf<Class<*>>(CaptureWidget::class.java, NeedsYouWidget::class.java, TasksWidget::class.java)

    /** Asks the launcher to put one of the three widgets on the home screen, which it confirms itself. */
    private fun addWidget(which: Int) {
        val ok = Widgets.pin(ctx(), widgetKinds[which])
        if (!ok) _state.update { it.copy(line = ctx().getString(R.string.settings_launcher_cannot)) }
    }

    /** Tries one saved item again now, then reads what is left. */
    private fun sendNow(id: String) {
        Outbox.of(ctx()).retry(id)
        viewModelScope.launch(Dispatchers.IO) {
            OutboxSender.flushNow(ctx())
            _state.update { it.copy(capture = captureFacts(), phone = phoneRows()) }
        }
    }

    private var upgraderShown = false

    private fun notifFacts(): NotifFacts {
        val nm = ctx().getSystemService(NotificationManager::class.java)
        val channels = listOf(AlertPoster.NEEDS, AlertPoster.UPDATES, AlertPoster.APP_UPDATES, AlertPoster.DOWNLOADS, AlertPoster.HUB).mapNotNull { nm.getNotificationChannel(it) }
            .map { ChannelFacts(it.id, it.name.toString(), it.importance, it.sound != null) }
        // Kept for as long as this screen lives, because the page lets the stored note go once it has been shown.
        val note = upgraderShown || AlertPoster.upgraderNote(ctx())
        upgraderShown = note
        return NotifFacts(
            nm.areNotificationsEnabled(), channels, AlertPoster.showDetails(ctx()), AlertPoster.deviceSecure(ctx()), note,
            needsImportance = nm.getNotificationChannel(AlertPoster.NEEDS)?.importance ?: IMPORTANCE_HIGH_FACT,
            server = ServerFacts(ServerHq.address(ctx()), ServerHq.signedOut(ctx()), ServerHq.keyLost(ctx()), example.hq.spike.ui.firstrun.FirstRunStore(ctx()).movedSeen()),
        )
    }

    /** Signs this phone in to the owner's server HQ with a one-time code; what went wrong is shown under the fields. */
    private fun connectServer(address: String, code: String) {
        if (_state.value.busy != null) return
        _state.update { it.copy(busy = SERVER_BUSY, serverError = null) }
        viewModelScope.launch {
            val problem = withContext(Dispatchers.IO) {
                try {
                    ServerHq.connect(ctx(), address, code)
                } catch (e: IllegalStateException) {
                    ctx().getString(R.string.settings_signin_not_kept)
                }
            }
            _state.update { it.copy(busy = null, serverError = problem, notif = notifFacts(), line = if (problem == null) ctx().getString(R.string.settings_signed_in_line) else it.line) }
        }
    }

    /** Stops asking the server HQ. The sign-in itself ends only when the phone is signed out there, which the line says. */
    private fun disconnectServer() {
        ServerHq.disconnect(ctx())
        _state.update { it.copy(serverError = null, notif = notifFacts(), line = ctx().getString(R.string.disconnect_server_done), askServerDisconnect = false) }
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
        if (!PhoneIntents.startFirst(ctx(), tries)) _state.update { it.copy(line = ctx().getString(R.string.settings_cannot_open_screen)) }
    }

    private fun phoneRow(key: String) {
        if (key == HQ_CODER_ROW) {
            _state.update { it.copy(open = false) }
            actions.onCodeFromPhone()
            return
        }
        if (key == WORK_TASKS_ROW || key == WORK_HELPERS_ROW) {
            _state.update { it.copy(open = false) }
            actions.onWork(key)
            return
        }
        val c = ctx()
        val intent = when (key) {
            "battery" -> Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
            "notifications" -> Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, c.packageName)
            else -> null
        }
        if (intent == null) {
            _state.update { it.copy(line = ctx().getString(R.string.settings_not_in_native)) }
            return
        }
        try { c.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } catch (e: android.content.ActivityNotFoundException) { _state.update { it.copy(line = ctx().getString(R.string.settings_cannot_open_screen)) } }
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
        try { HubService.switchMode(ctx(), toActive) } catch (e: java.io.IOException) { return saveFailedLine() }
        if (!waitForHq(before)) return restartTimeoutLine()
        if (HubService.childActive() == toActive) return null
        // The switch landed while hq was starting with the old mode: restart once more so it reads the file.
        val again = startStamp().second
        HubService.restartHq()
        return if (waitForHq(again)) null else restartTimeoutLine()
    }

    /** Stops hq and waits for it to answer again, with the mode left as it is. */
    private fun confirmRestart() {
        if (_state.value.restarting) return
        _state.update { it.copy(askRestart = false, restarting = true) }
        viewModelScope.launch {
            val said = withContext(Dispatchers.IO) {
                val before = startStamp().second
                HubService.restartHq()
                if (waitForHq(before)) ctx().getString(R.string.settings_hq_restarted) else restartTimeoutLine()
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
        _state.update { it.copy(line = ctx().getString(R.string.copy_details_copied)) }
    }

    private companion object {
        const val SILENCES_PATH = "/api/notifications/silences"
        const val SILENCE_ALL = "all"
        const val MODE_MAX = 64L
        private const val GOT_IT = "backup-got-it"
        const val EXIT_SCAN = 10
        const val HUB_CHANNEL = AlertPoster.HUB
        const val IMPORTANCE_HIGH_FACT = 4
        const val SAMSUNG_BATTERY_PACKAGE = "com.samsung.android.lool"
        const val SAMSUNG_BATTERY_PAGE = "com.samsung.android.sm.battery.ui.BatteryActivity"
        const val HTTP_NOT_FOUND = 404
        const val KEEP_CACHE_MS = 5_000L
    }

    private fun saveFailedLine() = ctx().getString(R.string.settings_save_failed)
    private fun restartTimeoutLine() = ctx().getString(R.string.settings_restart_timeout)
}

private const val TRUSTED_SITES = "trustedSites"
