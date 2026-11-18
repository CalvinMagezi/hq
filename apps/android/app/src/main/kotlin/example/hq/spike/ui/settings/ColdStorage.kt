package example.hq.spike.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.HqServices
import example.hq.spike.data.SinkInfo
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.launch

/**
 * Cold storage on the Backup page: the sinks that keep sealed originals away from this phone, whether each answers now, a way to
 * add one kept by a paired machine, and a check that a sink still opens. A folder sink needs Android's folder picker, so it is
 * made on a computer; the recovery code is shown once when a sink is made and typed again for a check, and nothing here keeps it.
 */
@Composable
internal fun ColdStorageCard() {
    val client = HqServices.get(LocalContext.current).client
    val scope = rememberCoroutineScope()
    var sinks by remember { mutableStateOf<Result<List<SinkInfo>>?>(null) }
    var version by remember { mutableIntStateOf(0) }
    LaunchedEffect(version) { sinks = runCatching { client.suiteSinks() } }
    var adding by remember { mutableStateOf(false) }
    var made by remember { mutableStateOf<Pair<String, String>?>(null) }
    var checking by remember { mutableStateOf<SinkInfo?>(null) }
    var result by remember { mutableStateOf<String?>(null) }

    Label(stringResource(R.string.cold_title))
    Group {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.cold_explain), style = HqType.BodySm, color = Mts.InkMuted)
            val list = sinks?.getOrNull()
            when {
                sinks == null -> Text(stringResource(R.string.cold_loading), style = HqType.BodySm, color = Mts.InkMuted)
                list == null -> Text(stringResource(R.string.cold_unavailable), style = HqType.BodySm, color = Mts.InkMuted)
                list.isEmpty() -> Text(stringResource(R.string.cold_empty), style = HqType.Body, color = Mts.Ink)
                else -> list.forEach { sink ->
                    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Column(Modifier.weight(1f)) {
                            Text(sink.id, style = HqType.BodyBold, color = Mts.Ink)
                            val kind = stringResource(if (sink.kind == "peer") R.string.cold_kind_peer else R.string.cold_kind_folder)
                            val state = stringResource(if (sink.reachable) R.string.cold_reachable else R.string.cold_unreachable)
                            Text("$kind · $state", style = HqType.BodySm, color = Mts.InkMuted)
                        }
                        TextButton({ checking = sink }, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.cold_check), style = HqType.BodyBold, color = Mts.BrandBlue) }
                    }
                }
            }
            result?.let { Text(it, style = HqType.BodySm, color = Mts.Ink) }
            TextButton({ adding = true }, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.cold_add), style = HqType.BodyBold, color = Mts.BrandBlue) }
        }
    }

    if (adding) {
        var name by remember { mutableStateOf("") }
        var addr by remember { mutableStateOf("127.0.0.1:5700") }
        var failed by remember { mutableStateOf(false) }
        var busy by remember { mutableStateOf(false) }
        AlertDialog(
            onDismissRequest = { adding = false },
            title = { Text(stringResource(R.string.cold_add_title), style = HqType.BodyBold, color = Mts.Ink) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(name, { name = it.lowercase(); failed = false }, label = { Text(stringResource(R.string.cold_name)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
                    OutlinedTextField(addr, { addr = it; failed = false }, label = { Text(stringResource(R.string.cold_addr)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
                    Text(stringResource(R.string.cold_addr_hint), style = HqType.Caption, color = Mts.InkMuted)
                    if (failed) Text(stringResource(R.string.cold_failed), style = HqType.BodySm, color = Mts.InkMuted)
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        busy = true
                        scope.launch {
                            runCatching { client.suiteAddPeerSink(name.trim(), addr.trim()) }
                                .onSuccess { code -> made = name.trim() to code; adding = false; version++ }
                                .onFailure { failed = true; busy = false }
                        }
                    },
                    enabled = !busy && name.isNotBlank() && addr.isNotBlank(),
                    modifier = Modifier.heightIn(min = 48.dp),
                ) { Text(stringResource(R.string.cold_make), style = HqType.BodyBold, color = Mts.BrandBlue) }
            },
            dismissButton = { TextButton({ adding = false }, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.cold_cancel), style = HqType.BodyBold, color = Mts.InkMuted) } },
            containerColor = Mts.Surface100,
        )
    }

    made?.let { (id, code) ->
        AlertDialog(
            onDismissRequest = {},
            title = { Text(stringResource(R.string.cold_code_title, id), style = HqType.BodyBold, color = Mts.Ink) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(code, style = HqType.BodyBold, color = Mts.Ink, fontFamily = FontFamily.Monospace, softWrap = false)
                    Text(stringResource(R.string.cold_code_body), style = HqType.BodySm, color = Mts.InkMuted)
                }
            },
            confirmButton = { TextButton({ made = null }, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.cold_code_done), style = HqType.BodyBold, color = Mts.BrandBlue) } },
            containerColor = Mts.Surface100,
        )
    }

    checking?.let { sink ->
        var code by remember(sink) { mutableStateOf("") }
        var failed by remember(sink) { mutableStateOf(false) }
        var busy by remember(sink) { mutableStateOf(false) }
        AlertDialog(
            onDismissRequest = { checking = null },
            title = { Text(stringResource(R.string.cold_check_title, sink.id), style = HqType.BodyBold, color = Mts.Ink) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(code, { code = it.uppercase(); failed = false }, label = { Text(stringResource(R.string.cold_code_label)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
                    if (failed) Text(stringResource(R.string.cold_check_failed), style = HqType.BodySm, color = Mts.InkMuted)
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        busy = true
                        scope.launch {
                            runCatching { client.suiteCheckSink(sink.id, code.trim()) }
                                .onSuccess { r -> result = r.text; checking = null }
                                .onFailure { failed = true; busy = false }
                        }
                    },
                    enabled = !busy && code.isNotBlank(),
                    modifier = Modifier.heightIn(min = 48.dp),
                ) { Text(stringResource(R.string.cold_check_run), style = HqType.BodyBold, color = Mts.BrandBlue) }
            },
            dismissButton = { TextButton({ checking = null }, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.cold_cancel), style = HqType.BodyBold, color = Mts.InkMuted) } },
            containerColor = Mts.Surface100,
        )
    }
}
