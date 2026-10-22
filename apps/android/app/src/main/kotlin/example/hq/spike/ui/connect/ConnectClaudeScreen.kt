package example.hq.spike.ui.connect

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import example.hq.spike.ui.pairing.Body
import example.hq.spike.ui.pairing.Notice
import example.hq.spike.ui.pairing.Page
import example.hq.spike.ui.pairing.PillButton
import example.hq.spike.ui.pairing.TopBar
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.delay

private const val TICK_MS = 1_000L

/** The Connect Claude screen: the address to give Claude, the arming window and who is connected. */
@Composable
fun ConnectClaudeHost(vm: ConnectViewModel, onClose: () -> Unit) {
    val ui by vm.ui.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var now by remember { mutableStateOf(System.currentTimeMillis() / MS_PER_SEC) }
    LaunchedEffect(Unit) { while (true) { now = System.currentTimeMillis() / MS_PER_SEC; delay(TICK_MS) } }
    BackHandler(onBack = onClose)
    Box(Modifier.fillMaxSize().background(Mts.Surface000).statusBarsPadding().navigationBarsPadding()) {
        Column(Modifier.fillMaxSize()) {
            TopBar(ConnectWords.TITLE, onClose)
            val facts = ui.facts
            Page {
                if (facts == null) {
                    Body("Asking HQ…")
                    return@Page
                }
                Body(ConnectWords.intro(facts))
                Spacer(Modifier.height(16.dp))
                Address(facts.url) { copy(context, it) }
                Text(ConnectWords.status(facts, now), style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.padding(top = 16.dp).testTag("connect-status"))
                Spacer(Modifier.height(16.dp))
                if (ConnectWords.armed(facts, now)) PillButton("Close the window", primary = false, enabled = !ui.busy, tag = "disarm") { vm.disarm() }
                else PillButton("Arm for ${ConnectWords.ARM_MINUTES} minutes", enabled = ConnectWords.canArm(facts) && !ui.busy, tag = "arm") { vm.arm() }
            }
        }
        ui.line?.let { Notice(it, Modifier.align(Alignment.BottomCenter), vm::dismissLine) }
    }
}

private const val MS_PER_SEC = 1_000L

@Composable
private fun Address(url: String?, onCopy: (String) -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp)) {
        Text("ADDRESS TO GIVE CLAUDE", style = HqType.Group, color = Mts.InkMuted)
        Text(url ?: "Not available yet", style = HqType.BodySm.copy(fontFamily = FontFamily.Monospace), color = if (url == null) Mts.Line else Mts.Ink, modifier = Modifier.testTag("connect-url"))
        if (url != null) PillButton("Copy the address", primary = false, tag = "copy-address") { onCopy(url) }
    }
}

private fun copy(context: Context, text: String) {
    context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Connector address", text))
}
