package example.hq.spike.ui.firstrun

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** The in-app notifications sheet: reason first, then Allow / Not now. */
@Composable
fun NotifUpgraderSheet(onAllow: () -> Unit, onClose: () -> Unit) {
    androidx.compose.ui.window.Dialog(onDismissRequest = onClose) {
        Column(
            Modifier.clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(stringResource(R.string.notify_title), style = HqType.H2, color = Mts.Ink)
            Text(stringResource(R.string.notify_body), style = HqType.BodySm, color = Mts.InkMuted)
            GreenPill(stringResource(R.string.notify_allow), onClick = onAllow)
            OutlinePill(stringResource(R.string.notify_not_now), onClick = onClose)
        }
    }
}
