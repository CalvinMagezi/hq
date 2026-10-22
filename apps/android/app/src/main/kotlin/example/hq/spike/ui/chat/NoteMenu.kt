package example.hq.spike.ui.chat

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import androidx.core.content.FileProvider
import example.hq.spike.ui.chat.blocks.Attachment
import example.hq.spike.ui.chat.blocks.NoteText
import example.hq.spike.ui.chat.blocks.rememberCopier
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private val Scrim = Color.Black.copy(alpha = 0.62f)
private val Small = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private val MenuTop = 150.dp
private val ExportTop = 70.dp

/** What the note menu and the export steps can ask for; Chat and Notes each supply their own. */
class NoteMenuActions(
    val onClose: () -> Unit = {},
    val onEdit: (() -> Unit)? = null,
    val onTogglePin: () -> Unit = {},
    val onAsk: () -> Unit = {},
    val onExport: () -> Unit = {},
    val onNotice: (String) -> Unit = {},
)

class ExportActions(
    val onFormat: (ExportFormat) -> Unit = {},
    val onClose: () -> Unit = {},
    val onAgain: () -> Unit = {},
    val onNotice: (String) -> Unit = {},
)

/** Everything you can do with the open note, in one place: pin, ask HQ, share, export, copy. Rename, move and delete wait for server routes. */
@Composable
fun NoteMenuSheet(sheet: NoteSheet, actions: NoteMenuActions) {
    val copy = rememberCopier()
    val context = LocalContext.current
    SheetFrame(actions.onClose, top = MenuTop) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp)) {
            Text(sheet.title, style = HqType.ScreenTitle, color = Mts.Ink, maxLines = 1)
            if (sheet.trail.isNotBlank()) Text(sheet.trail, style = Small, color = Mts.InkMuted)
        }
        Column(Modifier.verticalScroll(rememberScrollState())) {
            actions.onEdit?.let { ActionRow(HqIcons.Edit, "Edit", null, { it() }, minHeight = ROW_HEIGHT) }
            ActionRow(HqIcons.Pin, if (sheet.pinned) "Unpin" else "Pin to top", null, { actions.onTogglePin() }, minHeight = ROW_HEIGHT)
            ActionRow(HqIcons.Spark, "Ask HQ about this note", null, { actions.onClose(); actions.onAsk() }, minHeight = ROW_HEIGHT)
            ActionRow(HqIcons.Share, "Share", null, { actions.onClose(); shareText(context, sheet) }, minHeight = ROW_HEIGHT)
            ActionRow(HqIcons.Download, "Export as…", "PDF, Word, a picture and more", { actions.onClose(); actions.onExport() }, minHeight = ROW_HEIGHT)
            ActionRow(HqIcons.Copy, "Copy path", null, { copy(sheet.path); actions.onClose(); actions.onNotice("Copied the note path") }, minHeight = ROW_HEIGHT)
            ActionRow(HqIcons.Copy, "Copy note text", null, { copy(NoteText.withoutFrontmatter(sheet.body)); actions.onClose(); actions.onNotice("Copied") }, minHeight = ROW_HEIGHT)
        }
    }
}

private val ROW_HEIGHT = 64.dp

internal fun shareText(context: Context, sheet: NoteSheet) {
    val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_SUBJECT, sheet.title).putExtra(Intent.EXTRA_TEXT, NoteText.withoutFrontmatter(sheet.body))
    context.startActivity(Intent.createChooser(send, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

/** The export steps over the note: pick a format, wait with Cancel, then send or keep the file; a failure says what to do. */
@Composable
fun ExportOverlay(state: ExportState, actions: ExportActions) {
    when (state) {
        is ExportState.Choosing -> ExportChooser(state, actions)
        is ExportState.Making -> MakingDialog(state, actions)
        is ExportState.Ready -> ReadySheet(state, actions)
        is ExportState.Failed -> FailedDialog(state, actions)
    }
}

@Composable
private fun ExportChooser(state: ExportState.Choosing, actions: ExportActions) {
    SheetFrame(actions.onClose, top = ExportTop) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 4.dp)) {
            Text("Export as", style = HqType.H2, color = Mts.Ink)
            Text(state.note.title, style = Small, color = Mts.InkMuted)
        }
        Column(Modifier.verticalScroll(rememberScrollState())) {
            ExportGroup.entries.forEach { group ->
                val formats = ExportFormat.entries.filter { it.group == group }
                if (group == ExportGroup.Tables && state.note.tables == 0) return@forEach
                Text(group.label.uppercase(), style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold, letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 10.dp, end = 16.dp, bottom = 2.dp))
                formats.forEach { FormatRow(it) { actions.onFormat(it) } }
            }
        }
    }
}

@Composable
private fun FormatRow(format: ExportFormat, onClick: () -> Unit) {
    Row(
        Modifier.padding(horizontal = 8.dp).fillMaxWidth().heightIn(min = 56.dp).clip(RoundedCornerShape(16.dp)).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(format.label, style = HqType.ChipTitle.copy(lineHeight = 22.sp), color = Mts.Ink)
            Text(format.blurb, style = Small, color = Mts.InkMuted)
        }
        Text(format.ext, style = Small.copy(fontFamily = FontFamily.Monospace), color = Mts.InkMuted)
    }
}

@Composable
private fun Dialog(top: androidx.compose.ui.unit.Dp, onDismiss: () -> Unit, content: @Composable () -> Unit) {
    BackHandler(onBack = onDismiss)
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Scrim).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = {}))
        Column(Modifier.padding(start = 24.dp, top = top, end = 24.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp)) { content() }
    }
}

@Composable
private fun MakingDialog(state: ExportState.Making, actions: ExportActions) {
    Dialog(DIALOG_TOP, actions.onClose) {
        Text("Making your ${state.format.word}…", style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(bottom = 12.dp))
        Row(Modifier.padding(bottom = 20.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            example.hq.spike.ui.chat.Spinner(28)
            Text(
                if (state.format == ExportFormat.Pdf) "HQ is laying out the pages. Large notes take a little longer." else "HQ is making the file. Large notes take a little longer.",
                style = HqType.BodySm, color = Mts.InkMuted,
            )
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) { TextButton("Cancel", actions.onClose) }
    }
}

@Composable
private fun FailedDialog(state: ExportState.Failed, actions: ExportActions) {
    Dialog(DIALOG_TOP, actions.onClose) {
        Text("Could not export", style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(bottom = 12.dp))
        Text(state.reason, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(bottom = 20.dp))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
            TextButton("Close", actions.onClose)
            Box(
                Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button) { actions.onFormat(state.format) }.padding(horizontal = 24.dp),
                contentAlignment = Alignment.Center,
            ) { Text("Try again", style = HqType.BodySmBold, color = Mts.OnGreen) }
        }
    }
}

private val DIALOG_TOP = 300.dp

@Composable
private fun TextButton(label: String, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 24.dp), contentAlignment = Alignment.Center) {
        Text(label, style = HqType.BodySmBold, color = Mts.Ink)
    }
}

/** The finished file: Share is the green button, Save to Files opens the system file picker, Open shows it in a viewer. */
@Composable
private fun ReadySheet(state: ExportState.Ready, actions: ExportActions) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val save = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument(state.mime.ifBlank { "application/octet-stream" })) { uri ->
        if (uri != null) scope.launch {
            val ok = withContext(Dispatchers.IO) {
                try { context.contentResolver.openOutputStream(uri)?.use { out -> File(state.file).inputStream().use { it.copyTo(out) } }; true } catch (e: java.io.IOException) { false }
            }
            actions.onNotice(if (ok) "Saved to Files" else "Could not save the file")
        }
    }
    val kind = Attachment(state.name, "", state.mime, state.size)
    SheetFrame(actions.onClose, top = null) {
        Row(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(48.dp).background(Mts.Surface200, RoundedCornerShape(10.dp)), contentAlignment = Alignment.Center) { Icon(HqIcons.File, null, Modifier.size(26.dp), tint = Mts.Ink) }
            Column {
                Text(state.name, style = HqType.ScreenTitle, color = Mts.Ink, maxLines = 1)
                Text("${kind.kind} · ${kind.sizeWords}", style = Small, color = Mts.InkMuted)
            }
        }
        Column(Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp, bottom = 4.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            WideButton(HqIcons.Share, "Share", primary = true) { shareFile(context, state, actions) }
            WideButton(HqIcons.Download, "Save to Files", primary = false) { save.launch(state.name) }
            Box(Modifier.align(Alignment.CenterHorizontally).heightIn(min = 48.dp).clickable(role = Role.Button) { viewFile(context, state, actions) }.padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
                Text("Open", style = HqType.BodySmBold, color = Mts.BrandBlue)
            }
        }
        if (state.mime == "application/zip" && state.note.tables > 1) {
            Text("${state.note.tables} tables, saved together as one zip", style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 28.dp))
        } else {
            Box(Modifier.size(28.dp))
        }
    }
}

@Composable
private fun WideButton(icon: ImageVector, label: String, primary: Boolean, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(CircleShape).then(if (primary) Modifier.background(Mts.BrandGreen) else Modifier.border(1.dp, Mts.Line, CircleShape)).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 24.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(20.dp), tint = if (primary) Mts.OnGreen else Mts.Ink)
        Text(label, style = HqType.BodySmBold, color = if (primary) Mts.OnGreen else Mts.Ink)
    }
}

private fun uriOf(context: Context, state: ExportState.Ready) = FileProvider.getUriForFile(context, "${context.packageName}.files", File(state.file))

private fun shareFile(context: Context, state: ExportState.Ready, actions: ExportActions) {
    val send = Intent(Intent.ACTION_SEND).setType(state.mime.ifBlank { "*/*" }).putExtra(Intent.EXTRA_STREAM, uriOf(context, state)).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    try { context.startActivity(Intent.createChooser(send, null)) } catch (e: ActivityNotFoundException) { actions.onNotice("No app on this phone can take that file.") }
}

private fun viewFile(context: Context, state: ExportState.Ready, actions: ExportActions) {
    val view = Intent(Intent.ACTION_VIEW).setDataAndType(uriOf(context, state), state.mime.ifBlank { "*/*" }).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    try { context.startActivity(view) } catch (e: ActivityNotFoundException) { actions.onNotice("No app on this phone can open that file.") }
}
