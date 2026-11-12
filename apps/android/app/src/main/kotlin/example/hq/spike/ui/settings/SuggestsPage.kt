package example.hq.spike.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.clickable
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import example.hq.spike.ui.shell.ConfirmCopy
import example.hq.spike.ui.shell.ConfirmDialog
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private sealed interface Read {
    data object Loading : Read
    data object Unknown : Read
    data class Got(val summary: SuggestSummary) : Read
}

/**
 * What HQ suggests: how often it spoke up over the last 30 days and how each kind was answered, read from HQ, and a Clear.
 * An HQ without the route says "HQ has not said yet"; nothing here ever errors.
 */
@Composable
internal fun SuggestsPage() {
    val context = LocalContext.current
    val client = remember { HqServices.get(context).client }
    var read by remember { mutableStateOf<Read>(Read.Loading) }
    var ask by rememberSaveable { mutableStateOf(false) }
    val clear by SuggestsClear.state.collectAsState()
    val busy = clear == ClearState.Busy
    val cleared = clear == ClearState.Done
    suspend fun load() {
        read = try {
            SuggestFacts.parse(client.json("/api/offers/summary").toString())?.let { Read.Got(it) } ?: Read.Unknown
        } catch (e: HqUnavailable) {
            Read.Unknown
        }
    }
    LaunchedEffect(Unit) { load() }
    androidx.compose.runtime.DisposableEffect(Unit) { onDispose { SuggestsClear.reset() } }
    val days = (read as? Read.Got)?.summary?.days ?: SuggestFacts.DEFAULT_DAYS
    Page {
        Text(pluralStringResource(R.plurals.settings_suggests_intro, days, days), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp))
        Text(stringResource(R.string.settings_suggests_stays), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp))
        Group {
            when (val r = read) {
                Read.Loading -> Line(stringResource(R.string.settings_suggests_loading), null)
                Read.Unknown -> Line(stringResource(R.string.settings_suggests_unknown), null)
                is Read.Got -> if (r.summary.rows.isEmpty()) Line(pluralStringResource(R.plurals.settings_suggests_empty, r.summary.days, r.summary.days), null) else r.summary.rows.forEachIndexed { i, row ->
                    if (i > 0) Divider()
                    Line(row.says, stringResource(R.string.settings_suggests_line, row.shown, row.tapped, row.notNow, row.later, row.never))
                }
            }
        }
        // Clear is only offered when HQ answered: an HQ without the routes has nothing to forget.
        if (read is Read.Got) {
            Label(stringResource(R.string.settings_suggests_clear))
            Group {
                Column(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { SuggestsClear.reset(); ask = true }.padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.Center) {
                    Text(stringResource(R.string.settings_suggests_clear), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.settings_suggests_clear_sub), style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
        }
        if (cleared) Text(stringResource(R.string.settings_suggests_cleared), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp).semantics { liveRegion = LiveRegionMode.Polite })
    }
    // A finished clear closes the dialog and reads the list again; a failed one keeps it open with the reason.
    LaunchedEffect(clear) { if (clear == ClearState.Done) { ask = false; load() } }
    if (ask) {
        ConfirmDialog(
            ConfirmCopy.clearOffers(context), { ask = false },
            { SuggestsClear.start(client) },
            busy = busy, error = if (clear == ClearState.Failed) stringResource(R.string.settings_suggests_clear_failed) else null,
        )
    }
}

@Composable
private fun Line(title: String, detail: String?) {
    Column(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.Center) {
        Text(title, style = HqType.BodyBold, color = Mts.Ink)
        if (detail != null) Text(detail, style = HqType.BodySm, color = Mts.InkMuted)
    }
}
