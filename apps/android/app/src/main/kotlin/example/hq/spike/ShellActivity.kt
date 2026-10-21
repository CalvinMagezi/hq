package example.hq.spike

import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqStatus
import example.hq.spike.ui.chat.ChatOverlays
import example.hq.spike.ui.chat.ChatScreen
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

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        HqServices.ensureHubRunning(this)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        val start = intent.getStringExtra("place")?.let { name -> Place.entries.firstOrNull { it.name.equals(name, true) } } ?: Place.Chat
        val badge = intent.getIntExtra("badge", 0)
        val fixture = chatFixture(intent.getStringExtra("fixture"))
        val services = HqServices.get(this)
        fixture?.let(chat::freeze)
        setContent {
            val status by services.status.collectAsStateWithLifecycle()
            val chatState by chat.state.collectAsStateWithLifecycle()
            HqTheme {
                HqShell(
                    startAt = start, inboxBadge = badge, status = if (fixture != null) HqStatus.Live else status,
                    overlay = { ChatOverlays(chatState, chat.actions) },
                ) { place ->
                    if (place == Place.Chat) ChatScreen(chatState, chat.actions) else NotBuilt(place)
                }
            }
        }
    }
}
