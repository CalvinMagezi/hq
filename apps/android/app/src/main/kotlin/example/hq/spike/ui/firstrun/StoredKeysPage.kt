package example.hq.spike.ui.firstrun

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import example.hq.spike.HubService
import example.hq.spike.KeyVault
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Settings, "Keys on this phone": the names of the credentials sealed in the Android Keystore, never their values.
 * A key can be added, replaced or removed here; hq restarts so it reads the change.
 */
@Composable
fun StoredKeysPage() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var names by remember { mutableStateOf<List<String>?>(null) }
    var name by remember { mutableStateOf("") }
    var value by remember { mutableStateOf("") }
    var show by remember { mutableStateOf(false) }
    var confirming by remember { mutableStateOf<String?>(null) }
    var line by remember { mutableStateOf<String?>(null) }
    var bad by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }

    suspend fun reload() { names = withContext(Dispatchers.IO) { KeyVault.names(context) } }
    LaunchedEffect(Unit) { reload() }

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(FirstRunWords.KEYS_BODY, style = HqType.BodySm, color = Mts.InkMuted)
        val shape = RoundedCornerShape(12.dp)
        Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100)) {
            val listed = names
            if (listed != null && listed.isEmpty()) Text(FirstRunWords.KEYS_NONE, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
            listed?.forEachIndexed { i, n ->
                if (i > 0) Row(Modifier.fillMaxWidth().size(0.dp, 1.dp).background(Mts.Hairline)) {}
                Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(start = 16.dp, end = 4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(HqIcons.Key, null, Modifier.size(20.dp), tint = Mts.InkMuted)
                    Text(n, style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.weight(1f).padding(start = 8.dp))
                    if (confirming == n) {
                        TextAction(FirstRunWords.BACK, enabled = !busy) { confirming = null }
                        TextAction(FirstRunWords.KEYS_REMOVE, enabled = !busy) {
                            busy = true
                            scope.launch {
                                val ok = withContext(Dispatchers.IO) { KeyVault.remove(context, n).also { if (it) HubService.restartHq() } }
                                confirming = null
                                bad = !ok
                                line = if (ok) FirstRunWords.KEYS_REMOVED else FirstRunWords.KEYS_NOT_REMOVED
                                reload()
                                busy = false
                            }
                        }
                    } else {
                        TextAction("Replace", enabled = !busy) { name = n; line = null }
                        TextAction(FirstRunWords.KEYS_REMOVE, enabled = !busy) { confirming = n; line = null }
                    }
                }
            }
        }
        Text(FirstRunWords.KEYS_ADD, style = HqType.BodyBold, color = Mts.Ink)
        PlainField(FirstRunWords.KEYS_NAME_LABEL, name, FirstRunWords.KEYS_NAME_HINT, !busy) { name = it.trim(); line = null }
        SecretField(FirstRunWords.KEY_LABEL, value, FirstRunWords.KEY_HINT, show, !busy, bad = bad, onShow = { show = !show }) { value = it; line = null }
        line?.let { Message(it, good = !bad) }
        GreenPill(FirstRunWords.KEYS_SAVE, enabled = name.isNotEmpty() && value.isNotEmpty(), busy = busy) {
            val problem = KeyVault.problem(name, value) ?: KeyShape.keyProblem(value)
            if (problem != null) {
                bad = true
                line = "That key was not saved: $problem."
                return@GreenPill
            }
            busy = true
            val typedName = name
            val typedValue = value
            scope.launch {
                val refused = withContext(Dispatchers.IO) { KeyVault.put(context, typedName, typedValue).also { if (it == null) HubService.restartHq() } }
                bad = refused != null
                line = if (refused == null) FirstRunWords.KEYS_SAVED else "That key was not saved: $refused."
                if (refused == null) { name = ""; value = "" }
                reload()
                busy = false
            }
        }
    }
}
