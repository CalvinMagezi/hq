package example.hq.spike.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val FieldShape = RoundedCornerShape(24.dp)

/** The message box with Send. While HQ writes, Send turns into Stop in the same place. Enter adds a line; only the button sends. */
@Composable
fun Composer(state: ChatUiState, actions: ChatActions) {
    Row(
        Modifier.fillMaxWidth().background(Mts.Surface000).drawBehind { drawLine(Mts.Hairline, Offset.Zero, Offset(size.width, 0f), 1.dp.toPx()) }
            .padding(start = 8.dp, top = 8.dp, end = 8.dp, bottom = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.Bottom,
    ) {
        BasicTextField(
            value = state.draft, onValueChange = actions.onDraft, textStyle = HqType.Body.copy(color = Mts.Ink),
            cursorBrush = SolidColor(Mts.Ink), maxLines = MAX_LINES,
            modifier = Modifier.weight(1f).heightIn(min = 48.dp).clip(FieldShape).background(Mts.Surface100)
                .border(1.dp, Mts.Line, FieldShape).padding(horizontal = 16.dp, vertical = 11.dp),
            decorationBox = { inner ->
                Box {
                    if (state.draft.isEmpty()) Text("Message HQ", style = HqType.Body, color = Mts.InkMuted)
                    inner()
                }
            },
        )
        if (state.running) StopButton(actions.onStop) else SendButton(state.canSend, actions.onSend)
    }
}

@Composable
private fun SendButton(enabled: Boolean, onSend: () -> Unit) {
    Box(
        Modifier.size(48.dp).clip(CircleShape).background(if (enabled) Mts.BrandGreen else Mts.Surface200).alpha(if (enabled) 1f else 0.5f)
            .clickable(enabled = enabled, role = Role.Button, onClickLabel = "Send", onClick = onSend),
        contentAlignment = Alignment.Center,
    ) {
        Icon(HqIcons.Send, "Send", Modifier.size(24.dp), tint = if (enabled) Mts.OnGreen else Mts.InkMuted)
    }
}

@Composable
private fun StopButton(onStop: () -> Unit) {
    Box(
        Modifier.size(48.dp).clip(CircleShape).background(Mts.Surface200).border(1.dp, Mts.Line, CircleShape)
            .clickable(role = Role.Button, onClickLabel = "Stop", onClick = onStop),
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.size(16.dp).background(Mts.Ink, RoundedCornerShape(4.dp)))
    }
}

private const val MAX_LINES = 6
