package example.hq.spike.ui.connection

import androidx.annotation.StringRes
import androidx.compose.foundation.background
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.Connection
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** Chat's banner above the conversation: one component for every state but Connected (wave-4/01-offline.md P6). */
object ChatBanner {
    @StringRes fun words(c: Connection): Int? = when (c) {
        Connection.Connected -> null
        is Connection.Starting -> R.string.chat_bar_starting
        is Connection.Restarting -> R.string.chat_bar_restarting
        is Connection.Stopped -> R.string.chat_bar_stopped
        is Connection.Reconnecting -> R.string.chat_bar_reconnecting
        is Connection.Offline -> R.string.chat_bar_offline
        is Connection.AiNotAnswering -> R.string.chat_bar_ai_down
        is Connection.ServerUnreachable -> R.string.chat_bar_server
        Connection.ServerSignedOut -> R.string.chat_bar_signed_out
    }
}

@Composable
fun ChatBanner(status: Connection, onStartHq: () -> Unit, onAiKeys: () -> Unit) {
    val words = ChatBanner.words(status) ?: return
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).background(Mts.Surface100).padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        val icon = when (status) { is Connection.Stopped -> HqIcons.Stop; is Connection.Offline -> HqIcons.WifiOff; else -> HqIcons.Warning }
        Icon(icon, null, Modifier.size(20.dp), tint = Mts.Amber)
        Text(stringResource(words), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.weight(1f))
        when (status) {
            is Connection.Stopped -> TextButton(onStartHq) { Text(stringResource(R.string.conn_start_hq), style = HqType.BodyBold, color = Mts.BrandGreen) }
            is Connection.AiNotAnswering -> TextButton(onAiKeys) { Text(stringResource(R.string.chat_bar_ai_action), style = HqType.BodyBold, color = Mts.BrandGreen) }
            else -> Unit
        }
    }
}
