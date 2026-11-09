package example.hq.spike

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.LaunchedEffect
import kotlinx.coroutines.launch
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.tasks.Snack
import androidx.compose.runtime.produceState
import kotlinx.coroutines.delay
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import example.hq.spike.data.HqServices
import example.hq.spike.data.Connection
import example.hq.spike.ui.chat.ChatOverlays
import example.hq.spike.ui.chat.ChatScreen
import example.hq.spike.ui.shell.LocalGoTo
import example.hq.spike.ui.notes.NotesViewModel
import example.hq.spike.ui.notes.NotesUi
import example.hq.spike.ui.notes.NotesOverlays
import example.hq.spike.ui.helpers.HelperOverlays
import example.hq.spike.ui.helpers.HelpersScreen
import example.hq.spike.ui.helpers.HelpersUi
import example.hq.spike.ui.helpers.HelpersViewModel
import example.hq.spike.ui.helpers.helpersFixture
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.settings.SettingsHost
import example.hq.spike.ui.settings.SettingsPage
import example.hq.spike.ui.settings.SettingsViewModel
import example.hq.spike.ui.settings.settingsFixture
import example.hq.spike.ui.inbox.InboxCard
import example.hq.spike.ui.inbox.AllowedPage
import example.hq.spike.ui.inbox.InboxDetails
import example.hq.spike.ui.inbox.InboxScreen
import example.hq.spike.ui.inbox.SignOutAsk
import example.hq.spike.ui.firstrun.NotifUpgraderSheet
import example.hq.spike.ui.inbox.InboxUi
import example.hq.spike.ui.inbox.InboxViewModel
import example.hq.spike.ui.inbox.inboxFixture
import example.hq.spike.ui.notes.NotesHome
import example.hq.spike.ui.search.SearchGoes
import example.hq.spike.ui.search.SearchScreen
import example.hq.spike.ui.webhost.LocalOpenWeb
import example.hq.spike.ui.webhost.WebHostScreen
import example.hq.spike.ui.webhost.WebPage
import example.hq.spike.ui.search.SearchUi
import example.hq.spike.ui.search.SearchViewModel
import example.hq.spike.ui.tasks.QuickAddSheet
import example.hq.spike.ui.tasks.BriefingPage
import androidx.compose.foundation.layout.size
import androidx.compose.material3.IconButton
import androidx.compose.material3.Icon
import example.hq.spike.ui.theme.Mts
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.tasks.TasksLogic
import example.hq.spike.ui.tasks.TaskTab
import example.hq.spike.ui.tasks.TaskScope
import example.hq.spike.ui.tasks.TaskFilters
import example.hq.spike.ui.tasks.TaskFilterWords
import example.hq.spike.ui.tasks.TaskFilterSheet
import example.hq.spike.ui.tasks.TaskPage
import example.hq.spike.ui.tasks.TasksScreen
import example.hq.spike.ui.tasks.TasksUi
import example.hq.spike.ui.tasks.TasksViewModel
import example.hq.spike.ui.tasks.tasksFixture
import example.hq.spike.ui.notes.notesFixture
import example.hq.spike.ui.chat.ContextRef
import example.hq.spike.ui.chat.ChatUiState
import example.hq.spike.ui.chat.ChatViewModel
import example.hq.spike.ui.chat.chatFixture
import example.hq.spike.ui.shell.HqShell
import example.hq.spike.ui.shell.LinkTarget
import example.hq.spike.ui.shell.Links
import androidx.compose.runtime.remember
import example.hq.spike.ui.settings.PhoneFix
import example.hq.spike.ui.settings.PhoneIntents
import example.hq.spike.ui.startup.StartingScreen
import example.hq.spike.ui.startup.StartupWords
import example.hq.spike.ui.startup.Startup
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import example.hq.spike.ui.firstrun.FirstRunHost
import example.hq.spike.ui.firstrun.FirstRunLogic
import example.hq.spike.ui.firstrun.FirstRunStore
import example.hq.spike.ui.firstrun.FirstRunViewModel
import example.hq.spike.ui.firstrun.Step
import example.hq.spike.ui.startup.FirstLaunch
import example.hq.spike.ui.startup.StartRules
import example.hq.spike.ui.usage.ChipTarget
import example.hq.spike.ui.usage.UsageViewModel
import example.hq.spike.ui.usage.balanceFixture
import example.hq.spike.ui.shell.NotBuilt
import example.hq.spike.ui.shell.Place
import example.hq.spike.ui.theme.HqTheme

/**
 * The native shell and the launcher screen. The WebView app (MainActivity) stays as a fallback that only
 * adb opens: `adb shell am start -n example.hq.spike/.MainActivity` (never with -S: that force-stops the
 * app and hq with it). Extras for checking against the design, read only in a debuggable build:
 * `--es place notes`, `--ei badge 3`, `--es fixture empty|streaming|chats|offline|failed`.
 * This is the launcher, so any app can start it: a link extra is read only as a place to show, like LinkActivity's.
 */
class ShellActivity : ComponentActivity() {
    private val chat: ChatViewModel by viewModels()
    private val notes: NotesViewModel by viewModels()
    private val inbox: InboxViewModel by viewModels()
    private val tasks: TasksViewModel by viewModels()
    private val helpers: HelpersViewModel by viewModels()
    private val settings: SettingsViewModel by viewModels()
    private val usage: UsageViewModel by viewModels()
    private val search: SearchViewModel by viewModels()

    /** The lists a search matches, read when it opens; set from the composition that owns the states. */
    private var searchSources: () -> example.hq.spike.ui.search.SearchSources = { example.hq.spike.ui.search.SearchSources() }

    /** A result is open and Back should bring the search back, once. */
    private var resumeSearch by mutableStateOf(false)

    private fun openSearch(start: example.hq.spike.ui.search.SearchStart) {
        resumeSearch = false
        search.actions.onOpen(searchSources(), start)
    }

    /** Which first-run screen opens, decided in onCreate before hq starts (its first answer would mark the app as used). */
    private var firstRunStep: Step? = null
    private val firstRun: FirstRunViewModel by viewModels {
        object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T = FirstRunViewModel(application, firstRunStep) as T
        }
    }

    /** A place asked for by a tapped notification while the app is already open; the shell takes it and clears it. */
    private var requested by mutableStateOf<Place?>(null)

    private var notifFacts by mutableStateOf<NotifFactsNow?>(null)
    private var upgraderOpen by mutableStateOf(false)
    private var connectionOpen by mutableStateOf(false)
    private var turnOnOrigin = NotifOrigin.Later

    /** Set while the owner is in Android's settings for a Turn on, so coming back counts a grant. */
    private var inAndroidSettings: NotifOrigin? = null

    private val askNotifications = registerForActivityResult(androidx.activity.result.contract.ActivityResultContracts.RequestPermission()) { granted ->
        // Only an activity can say whether Android would show its dialog again.
        val rationale = !granted && shouldShowRequestPermissionRationale(android.Manifest.permission.POST_NOTIFICATIONS)
        NotifAccess.answered(this, turnOnOrigin, granted, rationale)
        if (!granted && turnOnOrigin == NotifOrigin.FirstRun) firstRun.actions.onNotifNotNow()
        rereadNotifications()
    }

    private fun rereadNotifications() {
        notifFacts = NotifAccess.read(this)
        inbox.actions.onServerOpen = { settings.actions.onOpen(); settings.actions.onPage(SettingsPage.Notifications) }
        settings.actions.onCodeFromPhone = { requested = Place.Helpers; helpers.actions.onGithubOpen() }
        inbox.refreshBanner()
    }

    /** The one Turn on: asks Android, or opens the page where the owner switches it on, as the phone's state requires. */
    private fun turnOnNotifications(origin: NotifOrigin) {
        turnOnOrigin = origin
        val c = packageName
        val intent = when (NotifAccess.next(NotifAccess.read(this))) {
            NotifStep.AskAndroid -> { askNotifications.launch(android.Manifest.permission.POST_NOTIFICATIONS); return }
            NotifStep.OpenApp -> Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, c)
            NotifStep.OpenChannel -> Intent(android.provider.Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS).putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, c).putExtra(android.provider.Settings.EXTRA_CHANNEL_ID, AlertPoster.NEEDS)
            NotifStep.Done -> { rereadNotifications(); return }
        }
        inAndroidSettings = origin
        try { startActivity(intent) } catch (e: android.content.ActivityNotFoundException) { inAndroidSettings = null }
    }

    override fun onStart() {
        super.onStart()
        example.hq.spike.update.AppVisibility.visible = true
    }

    override fun onStop() {
        example.hq.spike.update.AppVisibility.visible = false
        super.onStop()
    }

    override fun onResume() {
        super.onResume()
        rereadNotifications()
        Thread { example.hq.spike.update.Updater.onForeground(applicationContext) }.start()
        val origin = inAndroidSettings ?: return
        inAndroidSettings = null
        if (notifFacts?.needsOn == true) NotifAccess.grantedInSettings(this, origin)
    }

    /** The upgrader sheet: Android 13 and newer, the first open after an update, never asked and not allowed. Once only. */
    private fun decideUpgrader() {
        val f = NotifAccess.read(this)
        val info = packageManager.getPackageInfo(packageName, 0)
        @Suppress("DEPRECATION")
        val now = if (android.os.Build.VERSION.SDK_INT >= 28) info.longVersionCode.toInt() else info.versionCode
        val stored = NotifAccess.storedVersion(this).let { if (it == 0 && firstRunStep == null) 1 else it }
        upgraderOpen = NotifAccess.showUpgrader(f.sdk, stored, now, f.granted, f.asked, NotifAccess.upgraderShown(this))
        if (upgraderOpen) NotifAccess.markUpgraderShown(this)
        NotifAccess.saveVersion(this, now)
    }

    /** The web page shown in the host screen over everything, or null. */
    private var web by mutableStateOf<WebPage?>(null)

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (intent.getStringExtra(EXTRA_SHEET) == "connection") connectionOpen = true
        countNoticeTap(intent)
        val link = linkOf(intent) ?: return
        // Shared text is kept even when the pages cannot be closed: it joins the chat's draft, and nothing typed there is lost.
        val shared = example.hq.spike.capture.CaptureActivity.takeDraft()
        shared?.let(chat::addToDraft)
        if (hasUnsavedWork()) {
            Toast.makeText(this, if (shared != null) SHARED_KEPT else UNSAVED_WORK, Toast.LENGTH_LONG).show()
            return
        }
        closePages()
        requested = link.place
        applyLink(link)
    }

    /** The link an intent carries (an hq:// link, from a notification, a widget or another app), or the place a debug start names. */
    private fun linkOf(intent: Intent): LinkTarget? {
        intent.getStringExtra(Links.EXTRA)?.let { return Links.parse(it) }
        return debugExtra(intent, "place")?.let { name -> Place.entries.firstOrNull { it.name.equals(name, true) } }?.let { LinkTarget(it) }
    }

    /** Shows what a link names, over the place it belongs to. Waiting for what is not loaded yet is the view model's job. */
    private fun applyLink(t: LinkTarget) {
        when (t.place) {
            Place.Chat -> { t.id?.let(chat::openLinked); if (t.compose) chat.focusComposer() }
            Place.Inbox -> t.id?.let(inbox::openItem)
            Place.Tasks -> if (t.away) tasks.actions.onAway(true) else t.id?.let { tasks.openById(it) { } }
            Place.Notes -> { t.path?.let(notes::openNote); t.noteId?.let(notes::openNoteById); t.search?.let { openSearch(example.hq.spike.ui.search.SearchStart(it)) } }
            Place.Helpers -> t.id?.let(helpers::openSession)
        }
        t.web?.let { WebPage.fromSlug(it) }?.let { web = it }
        t.screen?.let { name ->
            val page = SettingsPage.entries.firstOrNull { it.name.equals(name, true) } ?: return@let
            settings.actions.onOpen()
            if (page != SettingsPage.Home) settings.actions.onPage(page)
        }
    }

    /** Closing the pages for a link would throw away what is being typed, and a link can come from another app. */
    private fun hasUnsavedWork(): Boolean {
        val tasksNow = tasks.state.value
        val typedInTask = tasksNow.detail?.drafts?.values?.any { it.isNotBlank() } == true
        val typedInQuickAdd = tasksNow.quick?.let { it.text.isNotBlank() || it.notes.isNotBlank() } == true
        return notes.state.value.write?.dirty == true || typedInTask || typedInQuickAdd
    }

    /** Pages and sheets open over the shell would hide the place a tapped notification asks for. */
    private fun closePages() {
        web = null
        search.actions.onClose()
        notes.closeAll()
        inbox.actions.onDetails(null)
        tasks.actions.onClose()
        tasks.actions.onQuick(null)
        helpers.actions.onClose()
        settings.close()
        chat.actions.onCloseNote()
        chat.actions.onClosePage()
        chat.actions.onCloseChats()
    }

    private fun openCapture() {
        startActivity(Intent(this, example.hq.spike.capture.CaptureActivity::class.java))
    }

    /** "Try again": make sure the service that runs hq is up and start counting the wait from now. */
    private fun tryStartAgain() {
        HqServices.get(applicationContext).restartWait()
        HqServices.ensureHubRunning(this)
    }

    private fun openBatterySettings() {
        // Samsung's own page first, then the battery optimisation list and app info: what stops an app is the optimisation, not Battery Saver.
        PhoneIntents.startFirst(this, PhoneIntents.tries(this, PhoneFix.SleepingApps).take(1) + PhoneIntents.tries(this, PhoneFix.Battery))
    }

    /** Puts the two lines on the screen, the build, the mode and the end of hq's log on the clipboard, with anything token-like blanked. */
    private fun copyStartDetails() {
        val active = (java.io.File(filesDir, "mode").takeIf { it.length() <= MODE_FILE_MAX }?.readText()?.trim() ?: "") == "active"
        val log = StartupWords.cleanLog(tailOf(java.io.File(filesDir, "hq.log")))
        val build = packageManager.getPackageInfo(packageName, 0).let { "${it.versionName.orEmpty().ifBlank { "hub" }} \u00b7 build ${if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.P) it.longVersionCode else it.versionCode.toLong()}" }
        val text = StartupWords.details(build, active, StartupWords.failedLines(example.hq.spike.ui.shell.Words.of(this), HubService.lastStart(this), HubService.lastExit(this), StartRules.STILL_OPENING_S), log)
        (getSystemService(CLIPBOARD_SERVICE) as android.content.ClipboardManager).setPrimaryClip(android.content.ClipData.newPlainText("HQ details", text))
        android.widget.Toast.makeText(this, "Copied", android.widget.Toast.LENGTH_SHORT).show()
    }

    private fun tailOf(f: java.io.File): List<String> = try {
        java.io.RandomAccessFile(f, "r").use { r ->
            val from = (r.length() - TAIL_BYTES).coerceAtLeast(0)
            r.seek(from)
            val bytes = ByteArray((r.length() - from).toInt()).also { r.readFully(it) }
            String(bytes, Charsets.UTF_8).lines().drop(if (from > 0) 1 else 0).filter { it.isNotBlank() }
        }
    } catch (e: java.io.IOException) { emptyList() }

    /** The first-run screen to open, or null when this install is already set up. Saves Welcome at once, so a kill resumes it. */
    private fun decideFirstRun(): Step? {
        val store = FirstRunStore(this)
        val saved = store.step()
        val heard = FirstLaunch.isDone(this)
        val needsKeyLookup = !store.done() && saved == null && !heard
        val step = FirstRunLogic.decide(store.done(), saved, needsKeyLookup && KeyVault.hasModelKey(this), heard)
        if (step != null && saved == null) store.save(step)
        return step
    }

    /** Only the app's own "HQ answered" notice carries the extra, so a shared link or a widget is never counted. */
    private fun countNoticeTap(intent: Intent) {
        if (intent.getBooleanExtra(AlertPoster.EXTRA_NOTICE_TAP, false)) example.hq.spike.measure.Counters.get(this).bump(example.hq.spike.measure.Counter.NoticeTapped)
    }

    private fun placeOf(intent: Intent): Place? = linkOf(intent)?.place

    /** The design-check extras (place, badge, fixture, balance) work only in a debuggable build: the shell is the launcher, so any app can start it with extras. */
    private fun debuggable(): Boolean = (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0

    private fun debugExtra(intent: Intent, name: String): String? = if (debuggable()) intent.getStringExtra(name) else null

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        web?.let { outState.putString(STATE_WEB, it.slug) }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Two launches in a row that never drew a frame: this one goes to the screen that saves the notes.
        if (debugExtra(intent, "fixture") == null && example.hq.spike.safe.CrashMarker.begin(filesDir)) {
            example.hq.spike.safe.SafeModeActivity.open(this)
            finish()
            return
        }
        window.decorView.post { example.hq.spike.safe.CrashMarker.drawn(filesDir) }
        // A rotation keeps the web page open; it loads again from its start.
        web = WebPage.fromSlug(savedInstanceState?.getString(STATE_WEB))
        if (debugExtra(intent, "fixture") == null) firstRunStep = decideFirstRun()
        HqServices.ensureHubRunning(this)
        notifFacts = NotifAccess.read(this)
        if (debugExtra(intent, "fixture") == null && savedInstanceState == null) decideUpgrader()
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        val start = placeOf(intent) ?: if (debugExtra(intent, "fixture")?.startsWith("n-") == true) Place.Notes else if (debugExtra(intent, "fixture")?.startsWith("i-") == true) Place.Inbox else if (debugExtra(intent, "fixture")?.startsWith("t-") == true) Place.Tasks else if (debugExtra(intent, "fixture")?.startsWith("h-") == true) Place.Helpers else Place.Chat
        val badge = if (debuggable()) intent.getIntExtra("badge", 0) else 0
        val fixtureName = debugExtra(intent, "fixture")
        // A rotation or a restore runs onCreate again with the launch intent; the link was applied the first time.
        if (savedInstanceState == null) {
            countNoticeTap(intent)
            linkOf(intent)?.let(::applyLink)
            example.hq.spike.capture.CaptureActivity.takeDraft()?.let(chat.actions.onDraft)
        }
        if (fixtureName == "a-alerts") sampleAlerts()
        val notesFixture = notesFixture(fixtureName)
        val inboxFixture = inboxFixture(fixtureName)
        val tasksFixture = tasksFixture(fixtureName)
        val helpersFixture = helpersFixture(fixtureName)
        val settingsFixture = settingsFixture(fixtureName)
        val fixture = chatFixture(fixtureName) ?: (notesFixture ?: inboxFixture ?: tasksFixture ?: helpersFixture ?: settingsFixture)?.let { ChatUiState() }
        val services = HqServices.get(this)
        inboxFixture?.let(inbox::freeze)
        tasksFixture?.let(tasks::freeze)
        helpersFixture?.let(helpers::freeze)
        settingsFixture?.let(settings::freeze)
        debugExtra(intent, "balance")?.let { usage.freeze(balanceFixture(it)) }
        if (notesFixture != null) notes.freeze(notesFixture) else fixture?.let(chat::freeze)
        setContent {
            val status by services.status.collectAsStateWithLifecycle()
            val chatState by chat.state.collectAsStateWithLifecycle()
            val notesState by notes.state.collectAsStateWithLifecycle()
            val inboxState by inbox.state.collectAsStateWithLifecycle()
            val tasksState by tasks.state.collectAsStateWithLifecycle()
            val helpersState by helpers.state.collectAsStateWithLifecycle()
            val settingsState by settings.state.collectAsStateWithLifecycle()
            val searchState by search.state.collectAsStateWithLifecycle()
            val balance by usage.chip.collectAsStateWithLifecycle()
            val shown: Connection = if (fixture == null) status else Connection.Connected
            searchSources = { example.hq.spike.ui.search.SearchSources(tasksState.tasks, chatState.chats, helpersState.cards, shown is Connection.ServerUnreachable || shown is Connection.ServerSignedOut) }
            val startupFixture = when (fixtureName) { "x-starting" -> Startup.Starting; else -> null }
            val firstDone = remember(status) { FirstLaunch.isDone(this@ShellActivity) }
            val firstRunUi by firstRun.ui.collectAsStateWithLifecycle()
            val coverRemoval = remember { example.hq.spike.update.Updater.startCoverRemoval(this@ShellActivity) }
            val startup = startupFixture ?: if (fixture == null) StartRules.decide(status, firstDone, coverRemoval) else Startup.None
            val freeBytes by services.freeBytes.collectAsStateWithLifecycle()
            val stopped = shown as? Connection.Stopped
            LaunchedEffect(stopped != null) {
                val now = System.currentTimeMillis()
                if (stopped != null && StartRules.autoOpenOk(StartRules.lastAutoOpen(this@ShellActivity), now)) { StartRules.markAutoOpen(this@ShellActivity, now); connectionOpen = true }
            }
            LaunchedEffect(chatState.failed, connectionOpen) { if (chatState.failed || connectionOpen) services.refreshAi() }
            val savedAt = remember(status) { StartRules.lastAnswer(this@ShellActivity).takeIf { it > 0 } }
            val uiScope = androidx.compose.runtime.rememberCoroutineScope()
            val atTop: (Place) -> Unit = { place ->
                if (shown !is Connection.Connected) connectionOpen = true
                else uiScope.launch {
                    when (place) {
                        Place.Notes -> notes.actions.onRefresh()
                        Place.Tasks -> tasks.refresh()
                        Place.Helpers -> helpers.refresh()
                        Place.Inbox -> inbox.refresh()
                        else -> Unit
                    }
                }
            }
            val notifUi = notifFacts?.let { NotifUi(it, ::turnOnNotifications) }
            HqTheme { CompositionLocalProvider(LocalOpenWeb provides { page -> web = page }, LocalNotif provides notifUi, example.hq.spike.ui.shell.LocalReselectAtTop provides atTop) {
                if (firstRunUi.step != null) FirstRunHost(firstRunUi, firstRun.actions)
                else if (startup == Startup.Starting) StartingScreen(((status as? Connection.Starting)?.step ?: 0) + 1, (status as? Connection.Starting)?.seconds ?: 12) { openCapture() }
                else HqShell(
                    onStatus = { connectionOpen = true },
                    strip = { place ->
                        val server = shown is Connection.ServerUnreachable || shown is Connection.ServerSignedOut
                        if (place != Place.Chat && (!server || place == Place.Inbox)) example.hq.spike.ui.connection.OfflineStrip(shown, savedAt, { connectionOpen = true }, ::tryStartAgain)
                        // A new version is said on every place, not only inside Settings.
                        example.hq.spike.ui.settings.UpdateLine(
                            settingsState,
                            onUpdate = { settings.actions.onOpen(); settings.actions.onPage(SettingsPage.Update) },
                            onNotNow = settings.actions.onUpdateNotNow,
                        )
                    },
                    startAt = start, inboxBadge = if (badge > 0) badge else inboxState.shownNeeds.size, status = shown,
                    onSearch = { openSearch(example.hq.spike.ui.search.SearchStart()) }, onOpenSearch = ::openSearch, onSettings = settings.actions.onOpen, updateDot = settingsState.updateDot,
                    subline = { place -> if (place == Place.Tasks && tasksState.tab == TaskTab.All) TaskFilterWords.bar(tasksState.scoped.count { TasksLogic.open(it) }) else null },
                    topActions = { place ->
                        if (place == Place.Tasks && tasksState.tab == TaskTab.All) IconButton({ tasks.actions.onFilter(true) }, Modifier.size(48.dp)) { Icon(HqIcons.Filter, "Filter", Modifier.size(24.dp), tint = if (tasksState.scope == TaskScope.Everything) Mts.Ink else Mts.BrandGreen) }
                    },
                    balance = if (shown !is Connection.Connected) null else balance,
                    onBalance = { target -> settings.actions.onOpen(); settings.actions.onPage(if (target == ChipTarget.Spending) SettingsPage.Spending else SettingsPage.Keys) },
                    request = requested, onRequestHandled = { requested = null },
                    aboveBar = { place -> if (place != Place.Tasks) tasksState.line?.let { Snack(it, tasks.actions, Modifier.align(Alignment.BottomCenter).padding(bottom = 4.dp)) } },
                    overlay = { ShellOverlays(chatState, notesState, inboxState, tasksState, helpersState); if (connectionOpen) ConnectionHost(shown, freeBytes); if (upgraderOpen) NotifUpgraderSheet(onAllow = { upgraderOpen = false; turnOnNotifications(NotifOrigin.Later) }, onClose = { upgraderOpen = false }); SettingsHost(settingsState, settings.actions); SearchHost(searchState, shown); if (tasksState.away.open) AwayHost(tasksState); web?.let { page -> WebHostScreen(page, onClose = { web = null }, onStartHq = ::tryStartAgain, onNotes = { web = null; requested = Place.Notes }) } },
                ) { place ->
                    when (place) {
                        Place.Chat -> ChatScreen(chatState, chat.actions, status = shown, onStartHq = ::tryStartAgain, onAiKeys = { settings.actions.onOpen(); settings.actions.onPage(SettingsPage.Keys) })
                        Place.Notes -> NotesPlace(notesState)
                        Place.Inbox -> InboxPlace(inboxState)
                        Place.Tasks -> TasksPlace(tasksState)
                        Place.Helpers -> HelpersScreen(helpersState, helpers.actions)
                        else -> NotBuilt(place)
                    }
                }
            } }
        }
    }

    private fun internetNow(): Int? {
        val cm = getSystemService(android.net.ConnectivityManager::class.java) ?: return null
        val caps = cm.getNetworkCapabilities(cm.activeNetwork ?: return null) ?: return null
        return when {
            caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI) || caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_ETHERNET) -> R.string.conn_net_wifi
            caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_CELLULAR) -> R.string.conn_net_mobile
            else -> null
        }
    }

    @androidx.compose.runtime.Composable
    private fun ConnectionHost(status: Connection, freeBytes: Long?) {
        val w = example.hq.spike.ui.shell.Words.of(this)
        val saved = StartRules.lastAnswer(this).takeIf { it > 0 }
        val facts = example.hq.spike.ui.connection.ConnectionFacts(
            freeBytes = freeBytes,
            savedAt = saved?.let { w.clock(java.time.Instant.ofEpochMilli(it)) },
            stoppedAtStep = (status as? Connection.Stopped)?.step?.plus(1),
            internet = internetNow(),
            details = StartupWords.failedLines(w, HubService.lastStart(this), HubService.lastExit(this), (status as? Connection.Stopped)?.let { (System.currentTimeMillis() - it.sinceMs) / 1000 } ?: StartRules.STILL_OPENING_S),
        )
        example.hq.spike.ui.connection.ConnectionSheet(
            status, facts,
            example.hq.spike.ui.connection.ConnectionActions(
                onStartHq = { connectionOpen = false; tryStartAgain() },
                onSaveCopy = { connectionOpen = false; example.hq.spike.safe.SafeModeActivity.open(this) },
                onAndroidSettings = { openBatterySettings() },
                onRestart = { connectionOpen = false; HubService.restartHq(); HqServices.get(applicationContext).restartWait() },
                onCopyDetails = ::copyStartDetails,
            ),
        ) { connectionOpen = false }
    }

    @androidx.compose.runtime.Composable
    private fun NotesPlace(notesState: NotesUi) {
        val go = LocalGoTo.current
        NotesHome(notesState, notes.actions, onAskHq = { go(Place.Chat); chat.actions.onDraft("Write me a note about ") })
    }

    /** Search sits over everything but a chosen result: the screen closes first, then the note, task or chat opens in its own place. */
    @androidx.compose.runtime.Composable
    private fun AwayHost(tasksState: TasksUi) {
        example.hq.spike.ui.tasks.AwayPage(tasksState.away, tasksState.tasks, tasks.actions, onTask = { id -> tasks.actions.onAway(false); requested = Place.Tasks; tasks.openById(id) { } })
    }

    @androidx.compose.runtime.Composable
    private fun SearchHost(searchState: SearchUi, shown: Connection) {
        val go = LocalGoTo.current
        // A result hides the search but keeps it; Back from that page brings it back once.
        fun shownResult(open: () -> Unit) { search.actions.onHide(); resumeSearch = true; open() }
        androidx.activity.compose.BackHandler(enabled = resumeSearch && !searchState.open && search.canResume) {
            resumeSearch = false
            notes.closeAll(); tasks.actions.onClose(); chat.actions.onCloseNote(); chat.actions.onClosePage(); helpers.actions.onClose()
            search.resume()
        }
        if (!searchState.open) return
        SearchScreen(
            searchState, search.actions,
            SearchGoes(
                onNote = { path -> shownResult { notes.openNote(path) } },
                onTask = { t -> shownResult { go(Place.Tasks); tasks.openById(t.id) { } } },
                onChat = { c -> shownResult { go(Place.Chat); chat.actions.onOpenChat(c.id) } },
                onHelper = { h -> shownResult { go(Place.Helpers); helpers.openSession(h.id) } },
                onAsk = { hits ->
                    search.actions.onHide(); resumeSearch = false
                    chat.startChatWith(hits.map { ContextRef(ContextRef.Kind.Note, it.path, it.title) })
                    go(Place.Chat)
                },
                onStartHq = { search.actions.onClose(); tryStartAgain() },
            ),
            stopped = shown is Connection.Stopped,
        )
    }

    /** Posts three sample notifications, for checking their words and buttons against the design with no hq. */
    private fun sampleAlerts() {
        fun item(id: String, title: String, body: String, actions: String, source: String? = null) = org.json.JSONObject(
            """{"id":"$id","kind":"action_needed","title":"$title","description":"$body","state":"pending","created_at":"${java.time.OffsetDateTime.now()}","metadata":{"extra":{"source_task":${if (source == null) "null" else "\"$source\""}}},"actions":[$actions]}""",
        )
        val items = linkedMapOf(
            "a1" to item("a1", "Harness session 'claude in hq-oss' is waiting for you", "Session s-1 on mac-mini is blocked on a prompt or approval:\\nDo you want to edit Chat.dc.html?\\nRead it with x", "\"approve\",\"reject\"", "session-supervisor"),
            "a2" to item("a2", "Delete 3 old backups?", "The nightly backup wants to remove the 3 oldest copies.", "\"approve\",\"reject\"", "backup-task"),
            "a3" to org.json.JSONObject("""{"id":"a3","kind":"value_item","title":"Backup finished","description":"2,860 notes saved to your Mac.","state":"pending","created_at":"${java.time.OffsetDateTime.now()}","actions":["dismiss"]}"""),
        )
        val nm = getSystemService(android.app.NotificationManager::class.java)
        AlertPoster.channels().forEach { nm.createNotificationChannel(it) }
        AlertPoster.post(this, items, items.keys.toList(), 5)
    }

    @androidx.compose.runtime.Composable
    private fun TasksPlace(tasksState: TasksUi) {
        val go = LocalGoTo.current
        TasksScreen(tasksState, tasks.actions, onHelpers = { go(Place.Helpers) })
    }

    @androidx.compose.runtime.Composable
    private fun InboxPlace(inboxState: InboxUi) {
        val go = LocalGoTo.current
        InboxScreen(inboxState, inbox.actions, onTakeLook = { card -> openFromInbox(card, go) })
    }

    /** "Check it" on a task waiting for review opens that task over the Inbox; any other card sends the owner to Helpers. */
    private fun openFromInbox(card: InboxCard, go: (Place) -> Unit) {
        when {
            card.kind == CardKind.TaskReview && card.taskRef != null -> tasks.openRef(card.taskRef) { go(Place.Tasks) }
            card.kind == CardKind.TaskReview -> go(Place.Tasks)
            card.sessionId != null -> { go(Place.Helpers); helpers.openSession(card.sessionId) }
            else -> go(Place.Helpers)
        }
    }

    /** What sits over the whole shell: Notes pages first (they cover the bars), then Chat's sheets and pages. */
    @androidx.compose.runtime.Composable
    private fun ShellOverlays(chatState: ChatUiState, notesState: NotesUi, inboxState: InboxUi, tasksState: TasksUi, helpersState: HelpersUi) {
        val go = LocalGoTo.current
        NotesOverlays(notesState, notes.actions) { items ->
            notes.closeAll()
            chat.startChatWith(items.map { (path, title) -> ContextRef(ContextRef.Kind.Note, path, title) })
            go(Place.Chat)
        }
        helpersState.followed?.let { thread ->
            LaunchedEffect(thread) {
                helpers.actions.onFollowedHandled()
                helpers.actions.onClose()
                // HQ named this chat itself, so it opens even if the list has not caught up or the chat was archived.
                chat.actions.onOpenChat(thread)
                go(Place.Chat)
            }
        }
        ChatOverlays(chatState, chat.actions) { path -> notes.openNote(path) }
        inboxState.details?.let { InboxDetails(it, inboxState.answering[it.id], inboxState.offline, inbox.actions) { card -> inbox.actions.onDetails(null); openFromInbox(card, go) } }
        SignOutAsk(inboxState, inbox.actions)
        inboxState.allowed?.let { a -> AllowedPage(a, inbox.actions) { thread -> inbox.actions.onAllowedDone(); chat.actions.onOpenChat(thread); go(Place.Chat) } }
        tasksState.detail?.let { TaskPage(it, tasksState.today, tasks.actions) }
        tasksState.quick?.let { QuickAddSheet(it, tasksState.today, tasksState.edge, tasks.actions) }
        if (tasksState.filterOpen) TaskFilterSheet(tasksState, tasks.actions)
        tasksState.briefing?.let { b ->
            BriefingPage(b, onClose = { tasks.actions.onBriefing(false) }, onRetry = { tasks.actions.onBriefing(true) }, onTask = { t -> tasks.actions.onBriefing(false); tasks.openById(t.id) { } }, onNote = { path -> tasks.actions.onBriefing(false); notes.openNote(path) })
        }
        HelperOverlays(helpersState, helpers.actions)
    }

    companion object {
        /** The running notice sets this to "connection" so a tap on a stopped HQ opens the sheet. */
        const val EXTRA_SHEET = "sheet"
        const val STATE_WEB = "web"
        const val MODE_FILE_MAX = 64L
        const val TAIL_BYTES = 16_384L
        const val UNSAVED_WORK = "Finish or discard what you are writing first"
        const val SHARED_KEPT = "Added to your chat message. Finish what you are writing first to open it."
    }
}
