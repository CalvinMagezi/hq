package example.hq.spike.ui.download

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.download.DlState
import example.hq.spike.download.DownloadWords
import example.hq.spike.download.Item
import example.hq.spike.download.Op
import example.hq.spike.ui.shell.rememberWords
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** One download: its state in words, a thin bar while it runs, and only the buttons that state has. */
@Composable
fun DownloadRow(item: Item, state: DlState, onOp: (Op) -> Unit, modifier: Modifier = Modifier) {
    val w = rememberWords()
    val m = DownloadWords.row(w, item, state)
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (m.spinner) CircularProgressIndicator(Modifier.size(20.dp), color = Mts.BrandBlue, strokeWidth = 2.dp)
            m.status?.let { Text(it, style = HqType.BodyBold, color = if (m.warn) Mts.Amber else Mts.Ink, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite }) }
        }
        m.progress?.let { p ->
            LinearProgressIndicator(progress = { p.coerceIn(0f, 1f) }, Modifier.fillMaxWidth().heightIn(max = 4.dp).clip(RoundedCornerShape(2.dp)).testTag("download-bar"), color = Mts.BrandBlue, trackColor = Mts.Surface200)
        }
        m.detail?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted) }
        if (m.buttons.isNotEmpty()) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                m.buttons.forEachIndexed { i, b ->
                    TextButton({ onOp(b.op) }, Modifier.heightIn(min = 48.dp).background(if (i == 0 && b.op != Op.Cancel && b.op != Op.Pause) Mts.Surface200 else Mts.Surface100, RoundedCornerShape(8.dp))) {
                        Text(b.label, style = HqType.BodyBold, color = if (b.op == Op.Cancel) Mts.DangerText else Mts.BrandBlue)
                    }
                }
            }
        }
    }
}
