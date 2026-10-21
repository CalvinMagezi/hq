package example.hq.spike

import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import example.hq.spike.data.HqServices
import example.hq.spike.ui.shell.HqShell
import example.hq.spike.ui.shell.Place
import example.hq.spike.ui.theme.HqTheme

/**
 * The native shell. It is not the launcher screen yet: the WebView app (MainActivity) stays the way in
 * until the places behind this bar are built. Open it with `adb shell am start -n example.hq.spike/.ShellActivity`.
 */
class ShellActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        HqServices.ensureHubRunning(this)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        val start = intent.getStringExtra("place")?.let { name -> Place.entries.firstOrNull { it.name.equals(name, true) } } ?: Place.Chat
        val badge = intent.getIntExtra("badge", 0)
        setContent { HqTheme { HqShell(startAt = start, inboxBadge = badge) } }
    }
}
