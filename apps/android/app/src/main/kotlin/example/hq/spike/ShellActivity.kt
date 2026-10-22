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
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.LaunchedEffect
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
import example.hq.spike.data.HqStatus
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
import example.hq.spike.ui.inbox.InboxDetails
import example.hq.spike.ui.inbox.InboxScreen
import example.hq.spike.ui.inbox.InboxUi
import example.hq.spike.ui.inbox.InboxViewModel
import example.hq.spike.ui.inbox.inboxFixture
import example.hq.spike.ui.notes.NotesHome
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
import example.hq.spike.ui.chat.ChatTime
import example.hq.spike.ui.chat.ChatViewModel
import example.hq.spike.ui.chat.chatFixture
import example.hq.spike.ui.shell.HqShell
import example.hq.spike.ui.shell.LinkTarget
import example.hq.spike.ui.shell.Links
import androidx.compose.runtime.remember
import example.hq.spike.ui.settings.PhoneFix
import example.hq.spike.ui.settings.PhoneIntents
import example.hq.spike.ui.startup.DidNotStartScreen
import example.hq.spike.ui.startup.StartingScreen
import example.hq.spike.ui.startup.StartupWords
import example.hq.spike.ui.startup.Startup
import example.hq.spike.ui.startup.FirstLaunch
import example.hq.spike.ui.usage.ChipTarget
import example.hq.spike.ui.usage.UsageViewModel
import example.hq.spike.ui.usage.balanceFixture
import example.hq.spike.ui.shell.NotBuilt
import example.hq.spike.ui.shell.Place
import example.hq.spike.ui.theme.HqTheme

/**
 * The native shell. It is not the launcher screen yet: the WebView app (MainActivity) stays the way in
 * until the places behind this bar are built. Open it with `adb shell am start -n example.hq.spike/.ShellActivity`
 * (never with -S: that force-stops the app and hq with it). Extras for checking against the design:
 * `--es place notes`, `--ei badge 3`, `--es fixture empty|streaming|chats|offline|failed`.
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

    /** A place asked for by a tapped notification while the app is already open; the shell takes it and clears it. */
    private var requested by mutableStateOf<Place?>(null)

    /** The web page shown in the host screen over everything, or null. */
    private var web by mutableStateOf<WebPage?>(null)

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        val link = linkOf(intent) ?: return
        // Shared text is kept even when the pages cannot be closed: it joins the chat's draft, and nothing typed there is lost.
        val shared = intent.getStringExtra(example.hq.spike.capture.CaptureActivity.EXTRA_DRAFT)
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
        return intent.getStringExtra("place")?.let { name -> Place.entries.firstOrNull { it.name.equals(name, true) } }?.let { LinkTarget(it) }
    }

    /** Shows what a link names, over the place it belongs to. Waiting for what is not loaded yet is the view model's job. */
    private fun applyLink(t: LinkTarget) {
        when (t.place) {
            Place.Chat -> t.id?.let(chat::openLinked)
            Place.Inbox -> t.id?.let(inbox::openItem)
            Place.Tasks -> t.id?.let { tasks.openById(it) { } }
            Place.Notes -> { t.path?.let(notes::openNote); if (t.search) notes.actions.onOpenSearch() }
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
        val text = StartupWords.details(build, active, StartupWords.failedLines(HubService.lastStart(this), HubService.lastExit(this)), log)
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

    private fun placeOf(intent: Intent): Place? = linkOf(intent)?.place

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        web?.let { outState.putString(STATE_WEB, it.slug) }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // A rotation keeps the web page open; it loads again from its start.
        web = WebPage.fromSlug(savedInstanceState?.getString(STATE_WEB))
        HqServices.ensureHubRunning(this)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        val start = placeOf(intent) ?: if (intent.getStringExtra("fixture")?.startsWith("n-") == true) Place.Notes else if (intent.getStringExtra("fixture")?.startsWith("i-") == true) Place.Inbox else if (intent.getStringExtra("fixture")?.startsWith("t-") == true) Place.Tasks else if (intent.getStringExtra("fixture")?.startsWith("h-") == true) Place.Helpers else Place.Chat
        val badge = intent.getIntExtra("badge", 0)
        val fixtureName = intent.getStringExtra("fixture")
        // A rotation or a restore runs onCreate again with the launch intent; the link was applied the first time.
        if (savedInstanceState == null) {
            linkOf(intent)?.let(::applyLink)
            intent.getStringExtra(example.hq.spike.capture.CaptureActivity.EXTRA_DRAFT)?.let(chat.actions.onDraft)
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
        intent.getStringExtra("balance")?.let { usage.freeze(balanceFixture(it)) }
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
            val offlineSince = chatState.offlineSince
            val now by produceState(System.currentTimeMillis(), offlineSince) {
                value = System.currentTimeMillis()
                while (offlineSince != null) { delay(STATUS_TICK_MS); value = System.currentTimeMillis() }
            }
            // A socket that has only just dropped (or not yet opened) is not worth a "Saved" line.
            val shown = when {
                status is HqStatus.Starting && fixture == null -> status
                offlineSince != null && now - offlineSince >= OFFLINE_GRACE_MS -> HqStatus.Saved(ChatTime.short(offlineSince, now))
                else -> HqStatus.Live
            }
            val startupFixture = when (fixtureName) { "x-starting" -> Startup.Starting; "x-failed" -> Startup.DidNotStart; else -> null }
            val firstDone = remember(status) { FirstLaunch.isDone(this@ShellActivity) }
            val startup = startupFixture ?: if (fixture == null) StartupWords.decide(status, firstDone) else Startup.None
            HqTheme { CompositionLocalProvider(LocalOpenWeb provides { page -> web = page }) {
                if (startup == Startup.Starting) StartingScreen((status as? HqStatus.Starting)?.seconds ?: 12) { openCapture() }
                else if (startup == Startup.DidNotStart) DidNotStartScreen(StartupWords.failedLines(HubService.lastStart(this@ShellActivity), HubService.lastExit(this@ShellActivity)), ::tryStartAgain, ::openBatterySettings, ::copyStartDetails)
                else HqShell(
                    startAt = start, inboxBadge = if (badge > 0) badge else inboxState.shownNeeds.size, status = shown,
                    onSearch = { search.actions.onOpen(tasksState.tasks, chatState.chats) }, onSettings = settings.actions.onOpen,
                    subline = { place -> if (place == Place.Tasks && tasksState.tab == TaskTab.All) TaskFilterWords.bar(tasksState.scoped.count { TasksLogic.open(it) }, tasksState.scope != TaskScope.Everything) else null },
                    topActions = { place ->
                        if (place == Place.Tasks && tasksState.tab == TaskTab.All) IconButton({ tasks.actions.onFilter(true) }, Modifier.size(48.dp)) { Icon(HqIcons.Filter, "Filter", Modifier.size(24.dp), tint = if (tasksState.scope == TaskScope.Everything) Mts.Ink else Mts.BrandGreen) }
                    },
                    balance = if (shown is HqStatus.Starting) null else balance,
                    onBalance = { target -> settings.actions.onOpen(); settings.actions.onPage(if (target == ChipTarget.Spending) SettingsPage.Spending else SettingsPage.Keys) },
                    request = requested, onRequestHandled = { requested = null },
                    aboveBar = { place -> if (place != Place.Tasks) tasksState.line?.let { Snack(it, tasks.actions, Modifier.align(Alignment.BottomCenter).padding(bottom = 4.dp)) } },
                    overlay = { ShellOverlays(chatState, notesState, inboxState, tasksState, helpersState); SettingsHost(settingsState, settings.actions); SearchHost(searchState); web?.let { page -> WebHostScreen(page) { web = null } } },
                ) { place ->
                    when (place) {
                        Place.Chat -> ChatScreen(chatState, chat.actions, starting = shown is HqStatus.Starting)
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

    @androidx.compose.runtime.Composable
    private fun NotesPlace(notesState: NotesUi) {
        val go = LocalGoTo.current
        NotesHome(notesState, notes.actions, onAskHq = { go(Place.Chat); chat.actions.onDraft("Write me a note about ") })
    }

    /** Search sits over everything but a chosen result: the screen closes first, then the note, task or chat opens in its own place. */
    @androidx.compose.runtime.Composable
    private fun SearchHost(searchState: SearchUi) {
        if (!searchState.open) return
        val go = LocalGoTo.current
        SearchScreen(
            searchState, search.actions,
            onNote = { path -> search.actions.onClose(); notes.openNote(path) },
            onTask = { t -> search.actions.onClose(); go(Place.Tasks); tasks.openById(t.id) { } },
            onChat = { c -> search.actions.onClose(); go(Place.Chat); chat.actions.onOpenChat(c.id) },
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
        tasksState.detail?.let { TaskPage(it, tasksState.today, tasks.actions) }
        tasksState.quick?.let { QuickAddSheet(it, tasksState.today, tasks.actions) }
        if (tasksState.filterOpen) TaskFilterSheet(tasksState, tasks.actions)
        tasksState.briefing?.let { b ->
            BriefingPage(b, onClose = { tasks.actions.onBriefing(false) }, onRetry = { tasks.actions.onBriefing(true) }, onTask = { t -> tasks.actions.onBriefing(false); tasks.openById(t.id) { } }, onNote = { path -> tasks.actions.onBriefing(false); notes.openNote(path) })
        }
        HelperOverlays(helpersState, helpers.actions)
    }

    private companion object {
        const val STATE_WEB = "web"
        const val STATUS_TICK_MS = 5_000L
        const val OFFLINE_GRACE_MS = 5_000L
        const val MODE_FILE_MAX = 64L
        const val TAIL_BYTES = 16_384L
        const val UNSAVED_WORK = "Finish or discard what you are writing first"
        const val SHARED_KEPT = "Added to your chat message. Finish what you are writing first to open it."
    }
}
