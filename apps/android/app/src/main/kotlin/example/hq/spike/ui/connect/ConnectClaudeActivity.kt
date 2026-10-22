package example.hq.spike.ui.connect

import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import example.hq.spike.data.HqServices
import example.hq.spike.ui.theme.HqTheme

/** Connect Claude, over the whole app. Closed to other apps like the shell (it needs a permission only `adb` holds). */
class ConnectClaudeActivity : ComponentActivity() {
    private val vm: ConnectViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        HqServices.ensureHubRunning(this)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        setContent { HqTheme { ConnectClaudeHost(vm, ::finish) } }
    }
}
