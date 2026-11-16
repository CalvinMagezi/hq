package example.hq.spike.ui.notes

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.HqServices
import example.hq.spike.data.SuiteFile
import example.hq.spike.data.SuiteRefused
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.launch

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
fun SuiteBar(source: SuiteSource, onNotice: (String) -> Unit) {
    val client = HqServices.get(LocalContext.current).client
    val scope = rememberCoroutineScope()
    var file by remember(source.path) { mutableStateOf<SuiteFile?>(null) }
    var busy by remember(source.path) { mutableStateOf(false) }
    var refusal by remember(source.path) { mutableStateOf<String?>(null) }
    val failed = stringResource(R.string.suite_bar_failed)

    LaunchedEffect(source.path) { file = runCatching { client.suiteFile(source.path) }.getOrNull() }

    fun run(call: suspend () -> SuiteFile) {
        if (busy) return
        busy = true
        refusal = null
        scope.launch {
            try {
                file = call()
            } catch (e: SuiteRefused) {
                refusal = e.message
            } catch (e: java.io.IOException) {
                onNotice(failed)
            } finally {
                busy = false
            }
        }
    }

    val f = file ?: return
    Column(Modifier.fillMaxWidth().background(Mts.Surface100).padding(start = 16.dp, end = 4.dp, top = 4.dp, bottom = 4.dp)) {
        Text(
            refusal ?: f.headline, style = HqType.BodySm, color = Mts.Ink,
            modifier = Modifier.padding(vertical = 8.dp, horizontal = 0.dp).semantics { liveRegion = LiveRegionMode.Polite },
        )
        if (refusal == null && f.canConfirm) f.details.forEach { Text(it, style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(bottom = 2.dp)) }
        if (refusal == null) Row(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
            when {
                busy -> Text(stringResource(R.string.suite_bar_working), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(vertical = 14.dp))
                f.canShrink -> BarButton(stringResource(R.string.suite_bar_make_smaller)) { run { client.suiteShrink(source.path) } }
                f.canConfirm -> {
                    BarButton(stringResource(R.string.suite_bar_move)) { run { client.suiteConfirm(f.job.orEmpty()) } }
                    if (f.canAbandon) BarButton(stringResource(R.string.suite_bar_not_now)) { run { client.suiteAbandon(f.job.orEmpty()) } }
                }
                f.canUndo -> BarButton(stringResource(R.string.suite_bar_undo)) { run { client.suiteUndo(f.job.orEmpty()) } }
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
