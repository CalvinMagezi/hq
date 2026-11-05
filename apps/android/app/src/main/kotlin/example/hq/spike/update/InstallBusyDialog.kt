package example.hq.spike.update

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import example.hq.spike.R
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** What an install that would stop work asks: the safe choice (wait) on top, the one that stops work below. Back is "Not now". */
@Composable
fun InstallBusyDialog(reason: BusyReason, onWait: () -> Unit, onStop: () -> Unit, onDismiss: () -> Unit) {
    val line = when (reason) {
        BusyReason.Chat -> R.string.update_busy_chat
        BusyReason.Backup -> R.string.update_busy_backup
        BusyReason.Outbox -> R.string.update_busy_outbox
        BusyReason.Download -> R.string.update_busy_download
    }
    Dialog(onDismissRequest = onDismiss) {
        Column(Modifier.clip(RoundedCornerShape(12.dp)).background(Mts.Surface200).padding(20.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.update_busy_title), style = HqType.ScreenTitle, color = Mts.Ink)
            Text(stringResource(line), style = HqType.Body, color = Mts.Ink)
            Text(stringResource(R.string.update_busy_stops), style = HqType.Body, color = Mts.InkMuted)
            TextButton(onWait, Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text(stringResource(R.string.update_busy_wait), style = HqType.BodyBold, color = Mts.BrandBlue) }
            TextButton(onStop, Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                Text(stringResource(if (reason == BusyReason.Chat) R.string.update_busy_stop_chat else R.string.update_busy_stop_other), style = HqType.BodyBold, color = Mts.DangerText)
            }
        }
    }
}
