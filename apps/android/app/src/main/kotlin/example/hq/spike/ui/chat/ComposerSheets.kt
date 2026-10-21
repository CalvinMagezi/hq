package example.hq.spike.ui.chat

import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.chat.blocks.BlockActions
import example.hq.spike.ui.chat.blocks.NoteRefs
import example.hq.spike.ui.chat.blocks.rememberCopier
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val SheetShape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
private val Scrim = Color.Black.copy(alpha = 0.62f)
private val PickerTop = 152.dp
private val LiftedTop = 190.dp
private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)

@Composable
private fun SheetFrame(onClose: () -> Unit, top: androidx.compose.ui.unit.Dp?, content: @Composable ColumnScope.() -> Unit) {
    BackHandler(onBack = onClose)
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Scrim).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClose))
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().then(if (top != null) Modifier.padding(top = top).fillMaxHeight() else Modifier)
                .clip(SheetShape).background(Mts.Surface100).imePadding().navigationBarsPadding(),
        ) {
            Box(Modifier.padding(top = 12.dp, bottom = 12.dp).size(width = 32.dp, height = 4.dp).align(Alignment.CenterHorizontally).background(Mts.Line, RoundedCornerShape(2.dp)))
            content()
        }
    }
}

/** "Add to this chat": a note or a task from a picker, a photo or a file from the system's own pickers. */
@Composable
fun AddSheet(actions: ChatActions) {
    val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { actions.onFiles(it) }
    val files = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { actions.onFiles(it) }
    SheetFrame(actions.onCloseAdd, top = null) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 28.dp)) {
            Text("Add to this chat", style = HqType.H2, color = Mts.Ink)
            Text("HQ reads what you add before it answers.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(top = 4.dp, bottom = 14.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Tile(HqIcons.Notes, "A note", Modifier.weight(1f)) { actions.onPickKind(ContextRef.Kind.Note) }
                Tile(HqIcons.Tasks, "A task", Modifier.weight(1f)) { actions.onPickKind(ContextRef.Kind.Task) }
            }
            Row(Modifier.padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Tile(HqIcons.Camera, "A photo", Modifier.weight(1f)) { actions.onCloseAdd(); photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) }
                Tile(HqIcons.File, "A file", Modifier.weight(1f)) { actions.onCloseAdd(); files.launch(arrayOf("*/*")) }
            }
        }
    }
}

@Composable
private fun Tile(icon: ImageVector, label: String, modifier: Modifier, onClick: () -> Unit) {
    Column(
        modifier.heightIn(min = 96.dp).clip(RoundedCornerShape(16.dp)).background(Mts.Surface200).clickable(role = Role.Button, onClick = onClick),
        horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
    ) {
        Icon(icon, null, Modifier.size(28.dp), tint = Mts.Ink)
        Text(label, style = HqType.BodySmBold, color = Mts.Ink)
    }
}

/** Search a note or a task and tap one to attach it to the next message. */
@Composable
fun PickerSheet(picker: Picker, actions: ChatActions) {
    val isNote = picker.kind == ContextRef.Kind.Note
    SheetFrame(actions.onClosePicker, top = PickerTop) {
        Text(if (isNote) "Pick a note" else "Pick a task", style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp))
        val shape = RoundedCornerShape(24.dp)
        BasicTextField(
            value = picker.query, onValueChange = actions.onPickerQuery, singleLine = true, textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 4.dp).fillMaxWidth().heightIn(min = 48.dp).clip(shape).background(Mts.Surface000).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp, vertical = 11.dp),
            decorationBox = { inner -> Box { if (picker.query.isEmpty()) Text(if (isNote) "Search your notes" else "Search your tasks", style = HqType.Body, color = Mts.InkMuted); inner() } },
        )
        val hint = when {
            picker.results.isNotEmpty() -> null
            picker.loading -> "Looking…"
            isNote && picker.query.isBlank() -> "Type a few words to find a note."
            else -> "Nothing matches that."
        }
        if (hint != null) Text(hint, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 16.dp, vertical = 16.dp))
        LazyColumn(Modifier.fillMaxWidth().weight(1f)) {
            items(picker.results, key = { it.ref }) { PickRow(it) { actions.onChoose(it) } }
        }
    }
}

@Composable
private fun PickRow(item: ContextRef, onClick: () -> Unit) {
    Row(
        Modifier.padding(horizontal = 8.dp).fillMaxWidth().heightIn(min = 64.dp).clip(RoundedCornerShape(16.dp)).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(40.dp).background(Mts.Surface200, RoundedCornerShape(10.dp)), contentAlignment = Alignment.Center) {
            Icon(if (item.kind == ContextRef.Kind.Note) HqIcons.Notes else HqIcons.Tasks, null, Modifier.size(22.dp), tint = Mts.Ink)
        }
        Column(Modifier.weight(1f)) {
            Text(item.label, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            val sub = if (item.kind == ContextRef.Kind.Note) NoteRefs.folderTrail(item.ref) else item.ref
            if (sub.isNotBlank()) Text(sub, style = HqType.BodySm, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** Long press on a message: the message lifted above the dimmed screen, and what can be done with it. */
@Composable
fun MessageActionsSheet(state: ChatUiState, message: UiMessage, blocks: BlockActions, actions: ChatActions) {
    BackHandler(onBack = actions.onCloseSheet)
    val copy = rememberCopier()
    val isReply = message.speaker == Speaker.Reply
    val time = ChatTime.clock(message.createdAt)
    val header = (if (isReply) "Reply" else "You") + if (time.isNotEmpty()) " · $time" else ""
    val canRegenerate = isReply && !state.running && state.messages.lastOrNull()?.id == message.id
    val canEdit = !isReply && !message.sending && !state.running
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Scrim).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = actions.onCloseSheet))
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.padding(top = LiftedTop, start = 16.dp, end = 16.dp, bottom = 12.dp).weight(1f).verticalScroll(rememberScrollState())) {
                val lift = Modifier.shadow(24.dp, RoundedCornerShape(20.dp), ambientColor = Color.Black, spotColor = Color.Black)
                if (isReply) ReplyBubble(message, blocks, lifted = true, modifier = lift) else UserBubble(message.text, false, message.files, modifier = lift)
            }
            Column(Modifier.fillMaxWidth().clip(SheetShape).background(Mts.Surface100).navigationBarsPadding().padding(bottom = 24.dp)) {
                Box(Modifier.padding(top = 12.dp, bottom = 12.dp).size(width = 32.dp, height = 4.dp).align(Alignment.CenterHorizontally).background(Mts.Line, RoundedCornerShape(2.dp)))
                Text(header.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(start = 24.dp, end = 24.dp, bottom = 8.dp))
                ActionRow(HqIcons.Copy, "Copy text", null) { copy(message.text); actions.onCloseSheet(); actions.onNotice("Copied") }
                ActionRow(HqIcons.TextCursor, "Select text", null) { actions.onSelectText(message.text) }
                if (canRegenerate) ActionRow(HqIcons.RotateCw, "Regenerate reply", "Runs the last question again. This reply is replaced.") { actions.onRegenerate(message.id) }
                if (canEdit) ActionRow(HqIcons.Edit, "Edit and resend", "Puts the question back in the box. Sending it replaces what came after.") { actions.onEditResend(message.id) }
            }
        }
    }
}

@Composable
private fun ActionRow(icon: ImageVector, label: String, subtitle: String?, onClick: () -> Unit) {
    Row(
        Modifier.padding(horizontal = 8.dp).fillMaxWidth().heightIn(min = if (subtitle == null) 56.dp else 64.dp).clip(RoundedCornerShape(16.dp)).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(24.dp), tint = Mts.Ink)
        Column(Modifier.weight(1f)) {
            Text(label, style = HqType.BodyBold, color = Mts.Ink)
            if (subtitle != null) Text(subtitle, style = HqType.BodySm, color = Mts.InkMuted)
        }
    }
}
