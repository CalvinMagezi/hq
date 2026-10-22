package example.hq.spike.ui.pairing

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import example.hq.spike.ui.theme.HqTheme

/**
 * Pairing, over the whole app. Closed to other apps (it needs a permission only `adb` holds); a link
 * from outside arrives through [PairLinkActivity], which hands over the text and nothing else.
 */
class PairingActivity : ComponentActivity() {
    private val vm: PairingViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
        )
        // A rotation runs onCreate again with the same intent; the link was shown the first time.
        if (savedInstanceState == null) handle(intent)
        setContent { HqTheme { PairingHost(vm, ::finish) } }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handle(intent)
    }

    private fun handle(intent: Intent) {
        intent.getStringExtra(EXTRA_LINK)?.let(vm::confirmLink)
        when (intent.getStringExtra(EXTRA_SCREEN)) {
            "show" -> vm.showCode()
            "scan" -> vm.go(PairingScreen.Scan)
            "paste" -> vm.go(PairingScreen.Paste)
        }
    }

    companion object {
        const val EXTRA_LINK = "link"
        /** `adb shell am start -n example.hq.spike/.ui.pairing.PairingActivity --es screen show` opens a screen directly. */
        const val EXTRA_SCREEN = "screen"
    }
}
