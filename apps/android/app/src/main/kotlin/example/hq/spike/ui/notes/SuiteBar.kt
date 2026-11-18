package example.hq.spike.ui.notes

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.HqServices
import example.hq.spike.data.SinkInfo
import example.hq.spike.data.SuiteFile
import example.hq.spike.data.SuiteRefused
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.launch
import example.hq.spike.data.suiteAbandon
import example.hq.spike.data.suiteConfirm
import example.hq.spike.data.suiteFile
import example.hq.spike.data.suiteOffload
import example.hq.spike.data.suiteShrink
import example.hq.spike.data.suiteSinks
import example.hq.spike.data.suiteUndo

/** What a note says about the file it was made from, read from its front matter. */
data class SuiteSource(val path: String)

/** The file an import or a shrink made this note from, if the note says so and the original may still exist. */
fun suiteSourceOf(noteText: String): SuiteSource? {
    val lines = noteText.lines()
    if (lines.firstOrNull()?.trim() != "---") return null
    val end = (1 until lines.size).firstOrNull { lines[it].trim() == "---" } ?: return null
    val keys = lines.subList(1, end).mapNotNull { l ->
        val i = l.indexOf(':')
        if (i <= 0 || l.first().isWhitespace()) null else l.substring(0, i).trim() to l.substring(i + 1).trim().trim('"', '\'')
    }.toMap()
    val source = keys["source"]?.takeIf { it.isNotBlank() && keys["source_sha256"]?.isNotBlank() == true } ?: return null
    return if (keys["suite_original_state"] == "gone") null else SuiteSource(source)
}

/**
 * One thin bar under the reader's top bar for a note made from an office file: make that file smaller, say yes or no to what
 * hq prepared, or undo. No page of its own: the question opens in place. hq writes every sentence about the file.
 */
@Composable
fun SuiteBar(source: SuiteSource, onNotice: (String) -> Unit, onChanged: () -> Unit) {
    val client = HqServices.get(LocalContext.current).client
    val scope = rememberCoroutineScope()
    var file by remember(source.path) { mutableStateOf<SuiteFile?>(null) }
    var busy by remember(source.path) { mutableStateOf(false) }
    var refusal by remember(source.path) { mutableStateOf<String?>(null) }
    val failed = stringResource(R.string.suite_bar_failed)
    var keeping by remember(source.path) { mutableStateOf<List<SinkInfo>?>(null) }
    var bringingBack by remember(source.path) { mutableStateOf(false) }

    LaunchedEffect(source.path) { file = runCatching { client.suiteFile(source.path) }.getOrNull() }

    fun run(call: suspend () -> SuiteFile) {
        if (busy) return
        busy = true
        refusal = null
        scope.launch {
            try {
                file = call()
                onChanged()
            } catch (e: SuiteRefused) {
                refusal = e.message
            } catch (e: java.io.IOException) {
                onNotice(failed)
            } finally {
                busy = false
            }
        }
    }

    val fixed = SuiteBarFixture.state
    val f = fixed?.first ?: file ?: return
    SuiteBarView(
        f, busy, fixed?.second ?: refusal,
        onShrink = { run { client.suiteShrink(source.path) } },
        onConfirm = { run { client.suiteConfirm(f.job.orEmpty()) } },
        onAbandon = { run { client.suiteAbandon(f.job.orEmpty()) } },
        onKeep = { scope.launch { keeping = runCatching { client.suiteSinks() }.getOrDefault(emptyList()).filter { it.reachable } } },
        onUndo = { if (f.originalInSink) bringingBack = true else run { client.suiteUndo(f.job.orEmpty()) } },
    )
    if (bringingBack) {
        RecoveryCodeDialog(
            title = stringResource(R.string.suite_undo_title),
            go = stringResource(R.string.suite_undo_go),
            onDismiss = { bringingBack = false },
            onGo = { code ->
                bringingBack = false
                run { client.suiteUndo(f.job.orEmpty(), code) }
            },
        )
    }
    keeping?.let { sinks ->
        KeepInSinkDialog(
            sinks,
            onDismiss = { keeping = null },
            onKeep = { sink, code ->
                keeping = null
                run { client.suiteOffload(f.job.orEmpty(), sink, code) }
            },
        )
    }
}

/** What a fixed screen (`--es fixture n-suite-ready`, `-prepared`, `-shrunk`, `-refused`) shows in the bar, so each state can be checked with no hq. */
object SuiteBarFixture {
    var state: Pair<SuiteFile, String?>? = null
}

/** The bar for a state, with no hq behind it. */
@Composable
@OptIn(ExperimentalLayoutApi::class)
fun SuiteBarView(f: SuiteFile, busy: Boolean, refusal: String?, onShrink: () -> Unit, onConfirm: () -> Unit, onAbandon: () -> Unit, onKeep: () -> Unit, onUndo: () -> Unit) {
    Column(Modifier.fillMaxWidth().background(Mts.Surface100).padding(start = 16.dp, end = 4.dp, top = 4.dp, bottom = 4.dp)) {
        Text(
            refusal ?: f.headline, style = HqType.BodySm, color = Mts.Ink,
            modifier = Modifier.padding(vertical = 8.dp, horizontal = 0.dp).semantics { liveRegion = LiveRegionMode.Polite },
        )
        if (refusal == null && f.canConfirm) f.details.forEach { Text(it, style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(bottom = 2.dp)) }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.Center) {
            when {
                busy -> Text(stringResource(R.string.suite_bar_working), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(vertical = 14.dp))
                f.canShrink -> BarButton(stringResource(R.string.suite_bar_make_smaller), onShrink)
                f.canConfirm -> {
                    BarButton(stringResource(R.string.suite_bar_move), onConfirm)
                    BarButton(stringResource(R.string.suite_bar_keep), onKeep)
                    if (f.canAbandon) BarButton(stringResource(R.string.suite_bar_not_now), onAbandon)
                }
                f.canUndo -> BarButton(stringResource(R.string.suite_bar_undo), onUndo)
            }
        }
    }
}

@Composable
private fun BarButton(label: String, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onClick).padding(end = 16.dp), contentAlignment = Alignment.CenterStart) {
        Text(label, style = HqType.BodySmBold, color = Mts.BrandBlue)
    }
}

@Composable
private fun KeepInSinkDialog(sinks: List<SinkInfo>, onDismiss: () -> Unit, onKeep: (String, String) -> Unit) {
    var chosen by remember { mutableStateOf(sinks.firstOrNull()?.id) }
    var code by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.suite_keep_title)) },
        text = {
            Column {
                if (sinks.isEmpty()) Text(stringResource(R.string.suite_keep_none), style = HqType.BodySm, color = Mts.InkMuted)
                sinks.forEach { s ->
                    BarButton((if (s.id == chosen) "● " else "○ ") + s.id) { chosen = s.id }
                }
                if (sinks.isNotEmpty()) OutlinedTextField(code, { code = it.uppercase() }, label = { Text(stringResource(R.string.cold_code_label)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            }
        },
        confirmButton = {
            TextButton(onClick = { chosen?.let { onKeep(it, code.trim()) } }, enabled = chosen != null && code.isNotBlank()) { Text(stringResource(R.string.suite_keep_go)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cold_cancel)) } },
    )
}

@Composable
private fun RecoveryCodeDialog(title: String, go: String, onDismiss: () -> Unit, onGo: (String) -> Unit) {
    var code by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = { OutlinedTextField(code, { code = it.uppercase() }, label = { Text(stringResource(R.string.cold_code_label)) }, singleLine = true, modifier = Modifier.fillMaxWidth()) },
        confirmButton = { TextButton(onClick = { onGo(code.trim()) }, enabled = code.isNotBlank()) { Text(go) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cold_cancel)) } },
    )
}
