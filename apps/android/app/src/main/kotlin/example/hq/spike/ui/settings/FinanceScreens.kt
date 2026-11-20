package example.hq.spike.ui.settings

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.finance.AndroidSmsReader
import example.hq.spike.finance.FinanceLastImport
import example.hq.spike.finance.FinancePrefs
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

/** "Finance": opt-in switch, READ_SMS permission ask (only shown here), and last import counts. */
@Composable
internal fun FinancePage() {
    val c = LocalContext.current.applicationContext
    val prefs = remember { FinancePrefs(c) }
    var enabled by remember { mutableStateOf(prefs.isEnabled()) }
    var allowed by remember { mutableStateOf(AndroidSmsReader.hasPermission(c)) }
    var asked by remember { mutableStateOf(false) }
    var summary by remember { mutableStateOf(prefs.lastSummary()) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        allowed = AndroidSmsReader.hasPermission(c)
        asked = true
    }
    Page {
        Text(
            stringResource(R.string.finance_intro),
            style = HqType.BodySm,
            color = Mts.InkMuted,
            modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp),
        )
        Group {
            FinanceToggleRow(enabled) { on ->
                prefs.setEnabled(on)
                enabled = on
                summary = prefs.lastSummary()
            }
        }
        if (allowed) {
            Notice(stringResource(R.string.finance_allowed_note), HqIcons.Check)
        } else {
            Notice(stringResource(if (asked) R.string.finance_denied_note else R.string.finance_need_access), HqIcons.Info, warn = asked)
            Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                CardButton(stringResource(R.string.finance_allow), true) { ask.launch(Manifest.permission.READ_SMS) }
            }
        }
        Label(stringResource(R.string.finance_last_import_label))
        Group { FinanceSummaryBody(summary) }
        Text(
            stringResource(R.string.finance_drafts_note),
            style = HqType.BodySm,
            color = Mts.InkMuted,
            modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp),
        )
    }
}

@Composable
private fun FinanceToggleRow(enabled: Boolean, onToggle: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(stringResource(R.string.finance_switch_title), style = HqType.BodyBold, color = Mts.Ink)
            Text(stringResource(R.string.finance_switch_note), style = HqType.BodySm, color = Mts.InkMuted)
        }
        Switch(
            checked = enabled,
            onCheckedChange = onToggle,
            colors = SwitchDefaults.colors(
                checkedTrackColor = Mts.BrandGreen,
                checkedThumbColor = Mts.OnGreen,
                checkedBorderColor = Mts.BrandGreen,
                uncheckedTrackColor = Mts.Surface200,
                uncheckedThumbColor = Mts.Line,
                uncheckedBorderColor = Mts.Line,
            ),
        )
    }
}

@Composable
private fun FinanceSummaryBody(summary: FinanceLastImport?) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (summary == null) {
            Text(stringResource(R.string.finance_no_import_yet), style = HqType.BodySm, color = Mts.InkMuted)
            return
        }
        val whenText = remember(summary.importedAtMs) {
            DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT)
                .withZone(ZoneId.systemDefault())
                .format(Instant.ofEpochMilli(summary.importedAtMs))
        }
        val counts = summary.counts
        Text(stringResource(R.string.finance_last_import_time, whenText), style = HqType.BodyBold, color = Mts.Ink)
        Text(
            stringResource(R.string.finance_counts_summary, counts.seen, counts.ignored, counts.transactions, counts.added, counts.waitingForReview),
            style = HqType.BodySm,
            color = Mts.InkMuted,
        )
    }
}
