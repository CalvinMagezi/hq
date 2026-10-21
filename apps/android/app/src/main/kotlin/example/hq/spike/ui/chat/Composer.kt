package example.hq.spike.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val FieldShape = RoundedCornerShape(24.dp)
private val ChipShape = RoundedCornerShape(12.dp)

/** The message box with Plus, Send and the chips above it. While HQ writes, Send turns into Stop in the same place. Enter adds a line; only the button sends. */
@Composable
fun Composer(state: ChatUiState, actions: ChatActions) {
    Column(
        Modifier.fillMaxWidth().background(Mts.Surface000).drawBehind { drawLine(Mts.Hairline, Offset.Zero, Offset(size.width, 0f), 1.dp.toPx()) }
            .padding(start = 8.dp, top = 8.dp, end = 8.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (state.files.isNotEmpty()) FileChips(state.files, actions)
        if (state.contexts.isNotEmpty() || state.editing != null) ItemChips(state, actions)
        MessageRow(state, actions)
    }
}

/** What is attached to the next message. HQ reads only what is shown here. */
@Composable
private fun ItemChips(state: ChatUiState, actions: ChatActions) {
    Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (state.editing != null) Chip(HqIcons.Edit, "Editing a message", "Stop editing", actions.onCancelEdit)
        state.contexts.forEach { item ->
            Chip(if (item.kind == ContextRef.Kind.Note) HqIcons.Notes else HqIcons.Tasks, item.label, "Remove ${item.label}") { actions.onRemoveContext(item) }
        }
    }
}

@Composable
private fun Chip(icon: ImageVector, label: String, removeLabel: String, onRemove: () -> Unit) {
    Row(
        Modifier.heightIn(min = 36.dp).widthIn(max = 220.dp).clip(CircleShape).background(Mts.Surface200).border(1.dp, Mts.Hairline, CircleShape).padding(start = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(16.dp), tint = Mts.Ink)
        Text(label, style = HqType.ToolLine, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
        Box(Modifier.size(36.dp).clickable(role = Role.Button, onClickLabel = removeLabel, onClick = onRemove).semantics { contentDescription = removeLabel }, contentAlignment = Alignment.Center) {
            Icon(HqIcons.Close, null, Modifier.size(16.dp), tint = Mts.Ink)
        }
    }
}

/** The files waiting to go with the message: a name, the size or what went wrong, and an x. Retry shows only when trying again can work. */
@Composable
private fun FileChips(files: List<PendingFile>, actions: ChatActions) {
    Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        files.forEach { file ->
            val failed = file.status == FileStatus.Failed
            Row(
                Modifier.heightIn(min = 48.dp).widthIn(max = 200.dp).clip(ChipShape).background(Mts.Surface100).border(1.dp, if (failed) Mts.DangerText else Mts.Hairline, ChipShape)
                    .then(if (file.retryable) Modifier.clickable(role = Role.Button, onClickLabel = "Try uploading ${file.name} again") { actions.onRetryFile(file.id) } else Modifier)
                    .padding(start = 8.dp, top = 4.dp, bottom = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(Modifier.size(32.dp).background(Mts.Surface200, RoundedCornerShape(6.dp)), contentAlignment = Alignment.Center) {
                    Icon(HqIcons.File, null, Modifier.size(18.dp), tint = Mts.Ink)
                }
                Column(Modifier.weight(1f, fill = false)) {
                    Text(file.name, style = HqType.ToolLine, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 110.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(fileLine(file), style = HqType.Caption, color = if (failed) Mts.DangerText else Mts.InkMuted, maxLines = 1)
                        if (file.retryable) Text("Retry", style = HqType.NavLabel, color = Mts.Ink)
                    }
                }
                Box(Modifier.size(36.dp).clickable(role = Role.Button, onClickLabel = "Remove ${file.name}") { actions.onRemoveFile(file.id) }.semantics { contentDescription = "Remove ${file.name}" }, contentAlignment = Alignment.Center) {
                    Icon(HqIcons.Close, null, Modifier.size(20.dp), tint = Mts.Ink)
                }
            }
        }
    }
}

private fun fileLine(file: PendingFile): String = when (file.status) {
    FileStatus.Uploading -> "Uploading…"
    FileStatus.Failed -> file.error ?: "upload failed"
    FileStatus.Ready -> sizeWords(file.size)
}

private fun sizeWords(size: Long): String = when {
    size < 0 -> ""
    size < 1024 -> "$size B"
    size < 1024 * 1024 -> "${Math.round(size / 1024.0)} KB"
    else -> "%.1f MB".format(java.util.Locale.US, size / (1024.0 * 1024.0))
}

@Composable
private fun MessageRow(state: ChatUiState, actions: ChatActions) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.Bottom) {
        Box(Modifier.size(48.dp).clip(CircleShape).clickable(role = Role.Button, onClickLabel = "Add to this chat", onClick = actions.onOpenAdd).semantics { contentDescription = "Add to this chat" }, contentAlignment = Alignment.Center) {
            Icon(HqIcons.Plus, null, Modifier.size(24.dp), tint = Mts.Ink)
        }
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

/** The commands offered while the field starts with a slash. It floats over the bottom of the conversation, on the composer's edge. */
@Composable
fun SlashPalette(draft: String, onPick: (String) -> Unit, modifier: Modifier = Modifier) {
    val shown = SlashCommands.matching(draft)
    if (shown.isEmpty()) return
    Column(modifier.padding(horizontal = 8.dp).fillMaxWidth().clip(ChipShape).background(Mts.Surface200).border(1.dp, Mts.Line, ChipShape)) {
        shown.forEach { (name, args, description) ->
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button) { onPick(name) }.padding(horizontal = 16.dp, vertical = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top,
            ) {
                Text(if (args.isEmpty()) name else "$name $args", style = HqType.BodySm.copy(fontFamily = FontFamily.Monospace), color = Mts.Ink)
                Text(description, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.weight(1f))
            }
        }
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
            .clickable(role = Role.Button, onClickLabel = "Stop", onClick = onStop).semantics { contentDescription = "Stop" },
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.size(16.dp).background(Mts.Ink, RoundedCornerShape(4.dp)))
    }
}

private const val MAX_LINES = 6
