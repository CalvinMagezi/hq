package example.hq.spike.ui.connection

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import example.hq.spike.R
import example.hq.spike.data.Connection
import example.hq.spike.data.ConnectionWords
import example.hq.spike.ui.firstrun.GreenPill
import example.hq.spike.ui.firstrun.OutlinePill
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.startup.StartRules
import example.hq.spike.ui.startup.StartSteps
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** What the sheet's buttons do. */
class ConnectionActions(
    val onStartHq: () -> Unit = {},
    val onSaveCopy: () -> Unit = {},
    val onAndroidSettings: () -> Unit = {},
    val onRestart: () -> Unit = {},
    val onCopyDetails: () -> Unit = {},
)

/** What the sheet shows besides the status: the saved-copy time, the Technical details lines, and a server HQ's address for an owner who has one. */
data class ConnectionFacts(
    val freeBytes: Long? = null,
    val savedAt: String? = null,
    val stoppedAtStep: Int? = null,
    val details: List<String> = emptyList(),
    val serverAddress: String? = null,
)

/** The Connection sheet (wave-2/04-startup.md frames 3 to 7): the state in words, the four steps while HQ starts, and the way out when it has stopped. */
@Composable
fun ConnectionSheet(status: Connection, facts: ConnectionFacts, actions: ConnectionActions, onClose: () -> Unit) {
    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Column(
            Modifier.padding(16.dp).heightIn(max = 640.dp).clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).verticalScroll(rememberScrollState()).padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            when (status) {
                is Connection.Stopped -> StoppedBody(facts, actions)
                is Connection.Starting -> StartingBody(status.step + 1, status.seconds, status.usualSeconds, facts, actions, onClose)
                is Connection.Restarting -> StartingBody(2, status.seconds, null, facts, actions, onClose)
                Connection.Connected -> {
                    Text(stringResource(R.string.state_connected), style = HqType.H2, color = Mts.Ink)
                    Text(stringResource(R.string.sheet_connected_body), style = HqType.BodySm, color = Mts.InkMuted)
                    facts.savedAt?.let { Fact(R.string.conn_fact_saved, it) }
                }
                else -> {
                    val words = ConnectionWords.of(status)
                    Text(stringResource(words.res, *words.args.toTypedArray()), style = HqType.H2, color = Mts.Ink)
                    ConnectionWords.reason(status)?.let { Text(stringResource(it), style = HqType.BodySm, color = Mts.InkMuted) }
                    ConnectionWords.outcome(status, true)?.let { Text(stringResource(it), style = HqType.BodySm, color = Mts.InkMuted) }
                    facts.savedAt?.let { Fact(R.string.conn_fact_saved, it) }
                }
            }
        }
    }
}

@Composable
private fun StartingBody(step: Int, seconds: Long, usual: Long?, facts: ConnectionFacts, actions: ConnectionActions, onClose: () -> Unit) {
    Text(stringResource(R.string.conn_title_starting), style = HqType.H2, color = Mts.Ink)
    Text(stringResource(R.string.conn_body_starting), style = HqType.BodySm, color = Mts.InkMuted)
    facts.serverAddress?.let { Fact(R.string.conn_fact_server, stringResource(R.string.conn_server_value, it)) }
    Fact(R.string.conn_fact_phone, stringResource(R.string.conn_phone_step, step, seconds))
    StartSteps(step, seconds, showCount = false)
    usual?.let { Text(stringResource(R.string.conn_usual, it), style = HqType.BodySm, color = Mts.InkMuted) }
    if (StartRules.slow(seconds, usual)) Amber(HqIcons.Clock, stringResource(R.string.conn_slow))
    FullLine(facts.freeBytes, actions)
    facts.savedAt?.let { Fact(R.string.conn_fact_saved, it) }
    if (StartRules.stillOpening(seconds, step)) {
        Text(stringResource(R.string.conn_still_opening), style = HqType.BodySm, color = Mts.Ink)
        GreenPill(stringResource(R.string.conn_keep_waiting), onClick = onClose)
        OutlinePill(stringResource(R.string.conn_restart), onClick = actions.onRestart)
    }
}

@Composable
private fun StoppedBody(facts: ConnectionFacts, actions: ConnectionActions) {
    Text(stringResource(R.string.notice_stopped), style = HqType.H2, color = Mts.Ink)
    Text(stringResource(R.string.conn_stopped_body), style = HqType.BodySm, color = Mts.InkMuted)
    facts.serverAddress?.let { Fact(R.string.conn_fact_server, stringResource(R.string.conn_server_value, it)) }
    Fact(R.string.conn_fact_phone, stringResource(R.string.conn_stopped_value))
    facts.stoppedAtStep?.let { Fact(R.string.conn_fact_stopped_at, stringResource(example.hq.spike.ui.startup.RunningNotice.stepName(it))) }
    facts.savedAt?.let { Fact(R.string.conn_fact_saved, it) }
    FullLine(facts.freeBytes, actions)
    GreenPill(stringResource(R.string.conn_start_hq), onClick = actions.onStartHq)
    OutlinePill(stringResource(R.string.safe_sheet_button), onClick = actions.onSaveCopy)
    OutlinePill(stringResource(R.string.conn_android_settings), onClick = actions.onAndroidSettings)
    Text(stringResource(R.string.conn_battery_hint), style = HqType.Caption, color = Mts.InkMuted)
    if (facts.details.isNotEmpty()) {
        Text(stringResource(R.string.conn_technical), style = HqType.BodySmBold, color = Mts.Ink)
        facts.details.forEach { Text(it, style = HqType.Caption.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp, lineHeight = 16.sp), color = Mts.InkMuted) }
        TextButton(actions.onCopyDetails, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.copy_details_title), style = HqType.BodySmBold, color = Mts.BrandBlue) }
    }
}

/** Under 200 MB free the sheet says so, in any start state. */
@Composable
private fun FullLine(freeBytes: Long?, actions: ConnectionActions) {
    if (!StartRules.almostFull(freeBytes)) return
    val needMb = ((StartRules.LOW_SPACE_BYTES - (freeBytes ?: 0L)) / MB).coerceAtLeast(1)
    Amber(HqIcons.Warning, stringResource(R.string.conn_almost_full, stringResource(R.string.dl_mb, needMb.toString())))
    OutlinePill(stringResource(R.string.conn_android_settings), onClick = actions.onAndroidSettings)
    Text(stringResource(R.string.conn_android_shows), style = HqType.Caption, color = Mts.InkMuted)
}

@Composable
private fun Amber(icon: androidx.compose.ui.graphics.vector.ImageVector, text: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(icon, null, Modifier.size(20.dp), tint = Mts.Amber)
        Text(text, style = HqType.BodySm, color = Mts.Amber)
    }
}

@Composable
private fun Fact(label: Int, value: String) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(stringResource(label), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.weight(1f, fill = false))
        Text(value, style = HqType.BodySmBold, color = Mts.Ink, modifier = Modifier.padding(start = 12.dp).weight(1f, fill = false))
    }
}

private const val MB = 1_000_000L
