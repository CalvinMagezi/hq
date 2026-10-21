package example.hq.spike

import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.runtime.getValue
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
import example.hq.spike.ui.inbox.InboxDetails
import example.hq.spike.ui.inbox.InboxScreen
import example.hq.spike.ui.inbox.InboxUi
import example.hq.spike.ui.inbox.InboxViewModel
import example.hq.spike.ui.inbox.inboxFixture
import example.hq.spike.ui.notes.NotesHome
import example.hq.spike.ui.notes.notesFixture
import example.hq.spike.ui.chat.ContextRef
import example.hq.spike.ui.chat.ChatUiState
import example.hq.spike.ui.chat.ChatTime
import example.hq.spike.ui.chat.ChatViewModel
import example.hq.spike.ui.chat.chatFixture
import example.hq.spike.ui.shell.HqShell
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

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        HqServices.ensureHubRunning(this)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        val start = intent.getStringExtra("place")?.let { name -> Place.entries.firstOrNull { it.name.equals(name, true) } } ?: if (intent.getStringExtra("fixture")?.startsWith("n-") == true) Place.Notes else if (intent.getStringExtra("fixture")?.startsWith("i-") == true) Place.Inbox else Place.Chat
        val badge = intent.getIntExtra("badge", 0)
        val fixtureName = intent.getStringExtra("fixture")
        if (fixtureName == "a-alerts") sampleAlerts()
        val notesFixture = notesFixture(fixtureName)
        val inboxFixture = inboxFixture(fixtureName)
        val fixture = chatFixture(fixtureName) ?: (notesFixture ?: inboxFixture)?.let { ChatUiState() }
        val services = HqServices.get(this)
        inboxFixture?.let(inbox::freeze)
        if (notesFixture != null) notes.freeze(notesFixture) else fixture?.let(chat::freeze)
        setContent {
            val status by services.status.collectAsStateWithLifecycle()
            val chatState by chat.state.collectAsStateWithLifecycle()
            val notesState by notes.state.collectAsStateWithLifecycle()
            val inboxState by inbox.state.collectAsStateWithLifecycle()
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
            HqTheme {
                HqShell(
                    startAt = start, inboxBadge = if (badge > 0) badge else inboxState.shownNeeds.size, status = shown,
                    onSearch = { notes.actions.onOpenSearch() },
                    overlay = { ShellOverlays(chatState, notesState, inboxState) },
                ) { place ->
                    when (place) {
                        Place.Chat -> ChatScreen(chatState, chat.actions)
                        Place.Notes -> NotesPlace(notesState)
                        Place.Inbox -> InboxPlace(inboxState)
                        else -> NotBuilt(place)
                    }
                }
            }
        }
    }

    @androidx.compose.runtime.Composable
    private fun NotesPlace(notesState: NotesUi) {
        val go = LocalGoTo.current
        NotesHome(notesState, notes.actions, onAskHq = { go(Place.Chat); chat.actions.onDraft("Write me a note about ") })
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
    private fun InboxPlace(inboxState: InboxUi) {
        val go = LocalGoTo.current
        InboxScreen(inboxState, inbox.actions, onTakeLook = { go(Place.Helpers) })
    }

    /** What sits over the whole shell: Notes pages first (they cover the bars), then Chat's sheets and pages. */
    @androidx.compose.runtime.Composable
    private fun ShellOverlays(chatState: ChatUiState, notesState: NotesUi, inboxState: InboxUi) {
        val go = LocalGoTo.current
        NotesOverlays(notesState, notes.actions) { items ->
            notes.closeAll()
            chat.startChatWith(items.map { (path, title) -> ContextRef(ContextRef.Kind.Note, path, title) })
            go(Place.Chat)
        }
        ChatOverlays(chatState, chat.actions) { path -> notes.openNote(path) }
        inboxState.details?.let { InboxDetails(it, inboxState.answering[it.id], inboxState.offline, inbox.actions) }
    }

    private companion object {
        const val STATUS_TICK_MS = 5_000L
        const val OFFLINE_GRACE_MS = 5_000L
    }
}
