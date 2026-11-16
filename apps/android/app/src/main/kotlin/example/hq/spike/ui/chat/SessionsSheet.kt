package example.hq.spike.ui.chat

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.WatchedSession
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val SheetShape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
private val SheetTop = 152.dp
private val ScrimColor = Color.Black.copy(alpha = 0.62f)
private val TouchTarget = 48.dp

/** The chat header's button: how many agents this chat watches, and how many of them wait on an answer. */
@Composable
fun SessionsButton(sessions: SessionsUi, onClick: () -> Unit) {
    val label = if (sessions.waiting > 0) stringResource(R.string.sessions_button_label_waiting, sessions.count, sessions.waiting)
    else stringResource(R.string.sessions_button_label, sessions.count)
    val text = if (sessions.waiting > 0) "${sessions.count} · ${sessions.waiting}" else sessions.count.toString()
    Row(
        Modifier.padding(end = 16.dp).heightIn(min = TouchTarget).clip(CircleShape).border(1.dp, Mts.Line, CircleShape)
            .clickable(role = Role.Button, onClick = onClick).semantics { contentDescription = label }.padding(horizontal = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.Eye, null, Modifier.size(20.dp), tint = Mts.InkMuted)
        Text(text, style = HqType.BodyBold, color = if (sessions.waiting > 0) Mts.BrandBlue else Mts.Ink)
    }
}

/** The agents this chat watches, each with a Drive switch. Tapping the dimmed part or pressing Back closes it. */
@Composable
fun SessionsSheet(sessions: SessionsUi, actions: ChatActions) {
    BackHandler(onBack = actions.onCloseSessions)
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(ScrimColor).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = actions.onCloseSessions))
        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(top = SheetTop).clip(SheetShape).background(Mts.Surface100).navigationBarsPadding()) {
            Box(Modifier.padding(top = 12.dp, bottom = 12.dp).size(width = 32.dp, height = 4.dp).align(Alignment.CenterHorizontally).background(Mts.Line, RoundedCornerShape(2.dp)))
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.sessions_title), style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                Box(Modifier.heightIn(min = TouchTarget).clickable(role = Role.Button, onClick = actions.onCloseSessions).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.close), style = HqType.BodyBold, color = Mts.Ink)
                }
            }
            Text(stringResource(R.string.sessions_drive_note), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
            if (sessions.sessions.isEmpty()) {
                Text(stringResource(R.string.sessions_empty), style = HqType.Body, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
            } else {
                LazyColumn(Modifier.fillMaxWidth()) {
                    items(sessions.sessions, key = { it.id }) { s -> SessionRow(s, busy = s.id in sessions.busy, actions) }
                }
            }
        }
    }
}

@Composable
private fun SessionRow(s: WatchedSession, busy: Boolean, actions: ChatActions) {
    val name = SessionsReducer.name(s, stringResource(R.string.sessions_unnamed))
    val status = stringResource(statusWords(s))
    val line = listOfNotNull(s.harness.takeIf { it.isNotBlank() && it != name }, s.taskDisplayId, status).joinToString(" · ")
    val switchLabel = stringResource(R.string.sessions_drive_label, name)
    val stopLabel = stringResource(R.string.sessions_stop_watching_named, name)
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(name, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(line, style = HqType.BodySm, color = if (s.needsAnswer) Mts.BrandBlue else Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                stringResource(R.string.sessions_stop_watching), style = HqType.BodySm, color = Mts.InkMuted,
                modifier = Modifier.heightIn(min = TouchTarget).clickable(enabled = !busy, role = Role.Button, onClick = { actions.onStopWatching(s.id) })
                    .semantics { contentDescription = stopLabel }.padding(top = 14.dp),
            )
        }
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Switch(
                checked = s.drive, onCheckedChange = { actions.onSessionDrive(s.id, it) }, enabled = !busy && (s.running || s.drive),
                modifier = Modifier.semantics { contentDescription = switchLabel },
                colors = SwitchDefaults.colors(
                    checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen, checkedBorderColor = Mts.BrandGreen,
                    uncheckedTrackColor = Mts.Surface200, uncheckedThumbColor = Mts.Line, uncheckedBorderColor = Mts.Line,
                ),
            )
            Text(stringResource(R.string.sessions_let_hq_drive), style = HqType.Caption, color = Mts.InkMuted)
        }
    }
}

private fun statusWords(s: WatchedSession): Int = when {
    !s.running -> R.string.sessions_status_unknown
    s.needsAnswer -> R.string.sessions_status_blocked
    s.agentStatus == "working" -> R.string.sessions_status_working
    s.agentStatus == "done" -> R.string.sessions_status_done
    s.agentStatus == "idle" -> R.string.sessions_status_idle
    else -> R.string.sessions_status_unknown
}

/** One line under the chat header while HQ drives at least one agent, so the owner can see it without opening the sheet. */
@Composable
fun DrivingStrip(sessions: SessionsUi, onClick: () -> Unit) {
    val driving = sessions.driving
    if (driving.isEmpty()) return
    val one = driving.singleOrNull()
    val text = when {
        one == null -> stringResource(R.string.sessions_driving_many, driving.size)
        one.taskDisplayId != null -> stringResource(R.string.sessions_driving_one, SessionsReducer.name(one, stringResource(R.string.sessions_unnamed)), one.taskDisplayId)
        else -> stringResource(R.string.sessions_driving_one_no_task, SessionsReducer.name(one, stringResource(R.string.sessions_unnamed)))
    }
    Row(
        Modifier.fillMaxWidth().heightIn(min = TouchTarget).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(8.dp).clip(CircleShape).background(Mts.BrandGreen))
        Text(text, style = HqType.BodySm, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
