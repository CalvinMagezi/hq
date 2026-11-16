package example.hq.spike.ui.chat

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
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.Connection
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** One-line bar above the composer for offline and signed-out states. */
object ProblemBar {
    sealed interface State {
        data object Offline : State
        data object SignedOut : State
    }

    fun forState(status: Connection): State? = when (status) {
        is Connection.Offline -> State.Offline
        Connection.ServerSignedOut -> State.SignedOut
        else -> null
    }
}

@Composable
fun ProblemBarView(state: ProblemBar.State, onSignIn: () -> Unit, modifier: Modifier = Modifier) {
    val line = when (state) {
        ProblemBar.State.Offline -> stringResource(R.string.chat_problem_offline)
        ProblemBar.State.SignedOut -> stringResource(R.string.chat_problem_signed_out)
    }
    val icon = if (state is ProblemBar.State.Offline) HqIcons.WifiOff else HqIcons.Warning
    Row(
        modifier.fillMaxWidth().heightIn(min = 48.dp).background(Mts.Surface100)
            .drawBehind { drawLine(Mts.Hairline, Offset.Zero, Offset(size.width, 0f), 1.dp.toPx()) }
            .padding(horizontal = 16.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(20.dp), tint = Mts.Amber)
        Text(line, style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.weight(1f).semantics { liveRegion = LiveRegionMode.Polite })
        if (state is ProblemBar.State.SignedOut) {
            TextButton(onSignIn, modifier = Modifier.heightIn(min = 48.dp)) {
                Text(stringResource(R.string.settings_sign_in), style = HqType.BodyBold, color = Mts.BrandGreen)
            }
        }
    }
}
