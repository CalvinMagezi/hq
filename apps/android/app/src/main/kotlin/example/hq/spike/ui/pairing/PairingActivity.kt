package example.hq.spike.ui.pairing

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import example.hq.spike.bridge.BridgeSupervisor
import example.hq.spike.bridge.BridgeWire
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
        diagnostics(intent)
        when (intent.getStringExtra(EXTRA_SCREEN)) {
            "show" -> vm.showCode()
            "scan" -> vm.go(PairingScreen.Scan)
            "paste" -> vm.go(PairingScreen.Paste)
        }
    }

    /**
     * For measuring with `adb` (this activity needs a permission only `adb` holds): `--es keep <device id>`
     * holds a connection to a paired device and traces its round trips into files/bridge.log, and
     * `--ez watch false` stops the app from reporting network changes, for a before and after.
     */
    private fun diagnostics(intent: Intent) {
        if (intent.hasExtra(EXTRA_WATCH)) BridgeSupervisor.watchNetwork = intent.getBooleanExtra(EXTRA_WATCH, true)
        intent.getStringExtra(EXTRA_KEEP)?.let { BridgeSupervisor.send(if (it.isBlank()) BridgeWire.keepStop() else BridgeWire.keep(it)) }
    }

    companion object {
        const val EXTRA_KEEP = "keep"
        const val EXTRA_WATCH = "watch"
        const val EXTRA_LINK = "link"
        /** `adb shell am start -n example.hq.spike/.ui.pairing.PairingActivity --es screen show` opens a screen directly. */
        const val EXTRA_SCREEN = "screen"
    }
}
