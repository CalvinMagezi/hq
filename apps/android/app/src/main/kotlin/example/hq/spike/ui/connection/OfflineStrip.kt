package example.hq.spike.ui.connection

import androidx.annotation.StringRes
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.Connection
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.Words
import example.hq.spike.ui.shell.rememberWords
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import java.time.Instant

/** The one line over a saved copy while HQ cannot answer (wave-4/01-offline.md P6). Only the stopped state has an action. */
object OfflineStrip {
    /** The lead sentence for a state, or null when no strip shows: Connected, Reconnecting and the AI state leave HQ itself fine. */
    @StringRes fun lead(c: Connection): Int? = when (c) {
        is Connection.Starting -> R.string.strip_lead_starting
        is Connection.Restarting -> R.string.strip_lead_restarting
        is Connection.Stopped -> R.string.strip_lead_stopped
        is Connection.Offline -> R.string.strip_lead_offline
        is Connection.ServerUnreachable -> R.string.strip_lead_server
        Connection.ServerSignedOut -> R.string.strip_lead_signed_out
        else -> null
    }

    /** The whole line: "HQ is starting. Saved on this phone at 14:32.", with the day in place of the time for a copy from before today. */
    fun line(w: Words, c: Connection, savedAtMs: Long?, nowMs: Long = System.currentTimeMillis()): String? {
        val lead = lead(c)?.let { w.s(it) } ?: return null
        if (savedAtMs == null || savedAtMs <= 0) return w.s(R.string.strip_line_bare, lead)
        val at = Instant.ofEpochMilli(savedAtMs)
        val today = w.clockOrDay(at, Instant.ofEpochMilli(nowMs)) == w.clock(at)
        val saved = w.s(if (today) R.string.strip_saved_at else R.string.strip_saved_on, w.clockOrDay(at, Instant.ofEpochMilli(nowMs)))
        return w.s(R.string.strip_line, lead, saved)
    }
}

@Composable
fun OfflineStrip(status: Connection, savedAtMs: Long?, onOpen: () -> Unit, onStartHq: () -> Unit, modifier: Modifier = Modifier) {
    val w = rememberWords()
    val line = OfflineStrip.line(w, status, savedAtMs) ?: return
    val stopped = status is Connection.Stopped
    val describe = w.s(R.string.strip_a11y, line)
    Row(
        modifier.fillMaxWidth().heightIn(min = 48.dp).background(Mts.Surface100)
            .clickable(role = Role.Button, onClickLabel = stringResource(R.string.conn_open_label), onClick = onOpen)
            .padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(if (stopped) HqIcons.Stop else HqIcons.Warning, null, Modifier.size(20.dp), tint = Mts.Amber)
        Text(line, style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.weight(1f))
        if (stopped) TextButton(onStartHq) { Text(stringResource(R.string.conn_start_hq), style = HqType.BodyBold, color = Mts.BrandGreen) }
    }
}
