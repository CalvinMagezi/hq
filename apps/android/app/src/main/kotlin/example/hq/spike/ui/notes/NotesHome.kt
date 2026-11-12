package example.hq.spike.ui.notes

import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.R
import example.hq.spike.data.HqClient.NoteItem
import example.hq.spike.ui.chat.blocks.NoteRefs
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
internal const val INBOX = "Notebooks/Inbox"

/** The Notes place: a search box, folders, pinned notes and the newest notes, with a button for a new one. */
@Composable
fun NotesHome(ui: NotesUi, actions: NotesActions, onAskHq: () -> Unit, onImport: (List<Uri>) -> Unit = {}) {
    LaunchedEffect(Unit) { actions.onRefresh() }
    DisposableEffect(Unit) {
        actions.onShown(true)
        onDispose { actions.onShown(false) }
    }
    val pickers = rememberVaultPickers(onImport, actions.onNotice)
    Box(Modifier.fillMaxSize()) {
        when {
            ui.loaded && ui.recent.isEmpty() && ui.pinned.isEmpty() && ui.folders.isEmpty() && !ui.offline && !ui.error -> EmptyNotes(actions, onAskHq)
            else -> NotesList(ui, actions)
        }
        if (ui.newOpen) NewSheet(actions, pickers)
        if (ui.foldersOpen) FoldersSheet(ui, actions)
    }
}

@Composable
private fun NotesList(ui: NotesUi, actions: NotesActions) {
    Box(Modifier.fillMaxSize()) {
        val list = androidx.compose.foundation.lazy.rememberLazyListState()
        example.hq.spike.ui.shell.ReselectEffect(example.hq.spike.ui.shell.Place.Notes, list)
        LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = PaddingValues(bottom = 96.dp)) {
            item { val open = example.hq.spike.ui.shell.LocalOpenSearch.current; SearchPill { open(example.hq.spike.ui.search.SearchStart(example.hq.spike.ui.search.SearchScope.Notes)) } }
            item { StateLine(ui, actions) }
            val chips = VaultRules.chips(ui.topFolders, ui.trashCount)
            item { FolderChips(chips, { actions.onOpenFolder("Notebooks/$it") }, actions.onOpenTrash) { actions.onMoreFolders(true) } }
            // Recent first: it answers "what was I just using"; Pinned follows and is gone when empty.
            if (ui.recent.isNotEmpty()) item { SectionLabel(stringResource(R.string.notes_home_recent)) }
            if (!ui.loaded) item { LoadingLine(ui.slow) }
            items(ui.recent, key = { it.path }) { NoteRow(it) { actions.onOpenNote(it.path) } }
            val pinned = VaultRules.pinnedShown(ui.pinned)
            if (pinned.isNotEmpty()) {
                item { SectionLabel(stringResource(R.string.notes_home_pinned)) }
                item {
                    LazyRow(contentPadding = PaddingValues(start = 16.dp, top = 4.dp, end = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        items(pinned, key = { it.path }) { PinnedCard(it) { actions.onOpenNote(it.path) } }
                    }
                }
            }
        }
        NewNoteButton(stringResource(R.string.vault_new), Modifier.align(Alignment.BottomEnd).padding(16.dp), onLongClick = { actions.onNew(INBOX) }) { actions.onNewSheet(true) }
    }
}

/** The one line that says why the list may be empty or old: still reading, HQ not answering, HQ answering badly. */
@Composable
private fun StateLine(ui: NotesUi, actions: NotesActions) {
    val said = ui.loaded && (ui.offline || ui.error)
    if (!said) return
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(stringResource(if (ui.error) R.string.vault_error_line else R.string.vault_offline_line), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.weight(1f))
        Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(role = Role.Button, onClick = actions.onRefresh).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
            Text(stringResource(R.string.notice_action_try_again), style = HqType.BodySmBold, color = Mts.BrandBlue)
        }
    }
}

@Composable
private fun LoadingLine(slow: Boolean) {
    Text(stringResource(if (slow) R.string.vault_still_reading else R.string.vault_getting_notes), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp))
}

@Composable
private fun SearchPill(onClick: () -> Unit) {
    val shape = RoundedCornerShape(26.dp)
    Row(
        Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp).fillMaxWidth().heightIn(min = 52.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape)
            .clickable(role = Role.Button, onClickLabel = stringResource(R.string.search_field_notes), onClick = onClick).padding(horizontal = 18.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.Search, null, Modifier.size(22.dp), tint = Mts.InkMuted)
        Text(stringResource(R.string.notes_search_your_notes), style = HqType.Body, color = Mts.InkMuted)
    }
}

/** At most six folders, a More chip when there are others, and Trash as the last chip with its count only when it holds something. */
@Composable
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
private fun FolderChips(chips: VaultRules.Chips, onOpen: (String) -> Unit, onTrash: () -> Unit, onMore: () -> Unit) {
    val large = androidx.compose.ui.platform.LocalDensity.current.fontScale >= VaultRules.LARGE_TEXT_SCALE
    val all: @Composable () -> Unit = {
        chips.shown.forEach { name -> Chip(HqIcons.Folder, name, stringResource(R.string.chat_open_file, name)) { onOpen(name) } }
        if (chips.hasMore) Chip(null, stringResource(R.string.vault_more_folders), androidx.compose.ui.res.pluralStringResource(R.plurals.vault_more_folders_label, chips.more.size, chips.more.size), onMore)
        val trash = stringResource(R.string.notes_trash)
        Chip(HqIcons.Trash, if (chips.trash > 0) "$trash ${chips.trash}" else trash, trash, onTrash)
    }
    // At large text the chips wrap onto more lines instead of running off the edge.
    if (large) androidx.compose.foundation.layout.FlowRow(Modifier.padding(start = 16.dp, top = 14.dp, end = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) { all() }
    else Row(Modifier.horizontalScroll(rememberScrollState()).padding(start = 16.dp, top = 14.dp, end = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) { all() }
}

@Composable
private fun Chip(icon: androidx.compose.ui.graphics.vector.ImageVector?, label: String, description: String, onClick: () -> Unit) {
    Row(
        Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClickLabel = description, onClick = onClick).padding(horizontal = 14.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) Icon(icon, null, Modifier.size(18.dp), tint = Mts.Ink)
        Text(label, style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1)
    }
}

@Composable
internal fun SectionLabel(text: String, count: Int? = null) {
    androidx.compose.foundation.layout.Row(
        Modifier.padding(start = 16.dp, top = 20.dp, end = 16.dp, bottom = 4.dp).semantics(mergeDescendants = true) { heading() },
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(text.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted)
        if (count != null) Text(example.hq.spike.ui.shell.Counts.badge(count), style = HqType.Group, color = Mts.InkMuted)
    }
}

@Composable
private fun PinnedCard(item: NoteItem, onClick: () -> Unit) {
    val shape = RoundedCornerShape(16.dp)
    Column(
        Modifier.size(width = 200.dp, height = 104.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 14.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(item.title.ifBlank { NoteRefs.title(item.path) }, style = HqType.ChipTitle, color = Mts.Ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
        val age = NoteTime.short(item.mtime).let { if (it == "now") "just now" else if (it.isEmpty()) "" else "$it ago" }
        Text(listOf(NoteRefs.folderTrail(item.path).substringBefore(" › "), age).filter { it.isNotBlank() }.joinToString(" · "), style = HqType.Caption, color = Mts.InkMuted, maxLines = 1)
    }
}

/** A note in a list: its name, how long ago it changed, and the folders it sits in. */
@Composable
internal fun NoteRow(item: NoteItem, onClick: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }
            .padding(horizontal = 16.dp, vertical = 12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(item.title.ifBlank { NoteRefs.title(item.path) }, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            Text(NoteTime.short(item.mtime), style = HqType.Caption.copy(lineHeight = 24.sp), color = Mts.InkMuted, modifier = Modifier.padding(start = 12.dp))
        }
        val trail = NoteRefs.folderTrail(item.path)
        val mark = VaultRules.markOf(item.review)
        val hq = VaultRules.writtenByHq(item)
        if (trail.isNotBlank() || hq || mark != ReviewMark.None) {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                if (trail.isNotBlank()) Text(trail, style = SmallText, color = Mts.BrandBlue, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                if (hq) Text(stringResource(R.string.vault_mark_hq), style = SmallText, color = Mts.InkMuted, maxLines = 1)
                if (mark == ReviewMark.New) NewMark()
                if (mark == ReviewMark.Changed) Text(stringResource(R.string.vault_mark_changed), style = SmallText, color = Mts.InkMuted, maxLines = 1)
            }
        }
    }
}

/** The quiet green dot and the word New; it goes when the note is opened. */
@Composable
private fun NewMark() {
    Row(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(8.dp).background(Mts.BrandGreen, CircleShape))
        Text(stringResource(R.string.vault_mark_new), style = SmallText, color = Mts.Ink, maxLines = 1)
    }
}

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
internal fun NewNoteButton(label: String, modifier: Modifier, onLongClick: (() -> Unit)? = null, onClick: () -> Unit) {
    val longLabel = stringResource(R.string.vault_create_empty)
    val haptic = androidx.compose.ui.platform.LocalHapticFeedback.current
    Row(
        modifier.heightIn(min = 56.dp).clip(CircleShape).background(Mts.BrandGreen)
            .combinedClickable(role = Role.Button, onLongClickLabel = longLabel, onLongClick = onLongClick?.let { go -> { haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress); go() } }, onClick = onClick).padding(start = 20.dp, end = 24.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.Plus, null, Modifier.size(24.dp), tint = Mts.OnGreen)
        Text(label, style = HqType.BodySmBold, color = Mts.OnGreen)
    }
}

/** The system pickers the New sheet opens: a document, and the camera writing to a file this app owns. */
class VaultPickers(val file: () -> Unit, val photo: () -> Unit)

@Composable
private fun rememberVaultPickers(onImport: (List<Uri>) -> Unit, onNotice: (String) -> Unit): VaultPickers {
    val context = androidx.compose.ui.platform.LocalContext.current
    val file = androidx.activity.compose.rememberLauncherForActivityResult(androidx.activity.result.contract.ActivityResultContracts.OpenMultipleDocuments()) { if (it.isNotEmpty()) onImport(it) }
    // The name survives a rotation or the app being recreated while the camera is open; the photo itself is in the cache directory.
    var shot by androidx.compose.runtime.saveable.rememberSaveable { androidx.compose.runtime.mutableStateOf<String?>(null) }
    val dir = { java.io.File(context.cacheDir, "captures").apply { mkdirs() } }
    fun uriOf(name: String) = androidx.core.content.FileProvider.getUriForFile(context, "${context.packageName}.files", java.io.File(dir(), name))
    val camera = androidx.activity.compose.rememberLauncherForActivityResult(androidx.activity.result.contract.ActivityResultContracts.TakePicture()) { ok ->
        val ended = PhotoShot(shot).finish(ok)
        shot = null
        ended.discard?.let { java.io.File(dir(), it).delete() }
        ended.handOver?.let { onImport(listOf(uriOf(it))) }
    }
    val noCamera = stringResource(R.string.vault_no_camera)
    val denied = stringResource(R.string.vault_camera_denied)
    val launchCamera = {
        val folder = dir()
        CaptureFiles.stale((folder.listFiles() ?: emptyArray()).map { it.name to it.lastModified() }, System.currentTimeMillis(), shot).forEach { java.io.File(folder, it).delete() }
        val name = "photo-${System.currentTimeMillis()}.jpg"
        shot = name
        try { camera.launch(uriOf(name)) } catch (e: android.content.ActivityNotFoundException) {
            PhotoShot(name).finish(false).discard?.let { java.io.File(folder, it).delete() }
            shot = null
            onNotice(noCamera)
        }
    }
    // The manifest declares CAMERA (the pairing scanner uses it), so the camera app refuses to open for us until it is granted.
    val ask = androidx.activity.compose.rememberLauncherForActivityResult(androidx.activity.result.contract.ActivityResultContracts.RequestPermission()) { granted -> if (granted) launchCamera() else onNotice(denied) }
    return androidx.compose.runtime.remember(file, camera, ask) {
        VaultPickers(
            file = { file.launch(arrayOf("*/*")) },
            photo = {
                val held = androidx.core.content.ContextCompat.checkSelfPermission(context, android.Manifest.permission.CAMERA) == android.content.pm.PackageManager.PERMISSION_GRANTED
                if (held) launchCamera() else ask.launch(android.Manifest.permission.CAMERA)
            },
        )
    }
}

/** New: four rows. Import and Take a photo hand the file to the chat, where HQ reads it and writes the note. */
@Composable
private fun NewSheet(actions: NotesActions, pickers: VaultPickers) {
    val close = { actions.onNewSheet(false) }
    val clipboard = androidx.compose.ui.platform.LocalClipboardManager.current
    example.hq.spike.ui.chat.SheetFrame(close, top = null) {
        Text(stringResource(R.string.vault_new_in), style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp))
        example.hq.spike.ui.chat.ActionRow(HqIcons.Edit, stringResource(R.string.quick_note_title), stringResource(R.string.vault_new_note_hint), { close(); actions.onNew(INBOX) })
        example.hq.spike.ui.chat.ActionRow(HqIcons.File, stringResource(R.string.vault_import_file), stringResource(R.string.vault_import_file_hint), { close(); pickers.file() })
        example.hq.spike.ui.chat.ActionRow(HqIcons.Camera, stringResource(R.string.vault_take_photo), stringResource(R.string.vault_take_photo_hint), { close(); pickers.photo() })
        example.hq.spike.ui.chat.ActionRow(HqIcons.Paste, stringResource(R.string.vault_paste), stringResource(R.string.vault_paste_hint), { actions.onPasteNew(clipboard.getText()?.text.orEmpty()) })
        Box(Modifier.size(12.dp))
    }
}

/** Every folder under the Vault, for the More chip. */
@Composable
private fun FoldersSheet(ui: NotesUi, actions: NotesActions) {
    val close = { actions.onMoreFolders(false) }
    example.hq.spike.ui.chat.SheetFrame(close, top = 150.dp) {
        Text(stringResource(R.string.notes_folders), style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp))
        LazyColumn(Modifier.fillMaxWidth().weight(1f)) {
            items(ui.topFolders, key = { it }) { name -> example.hq.spike.ui.chat.ActionRow(HqIcons.Folder, name, null, { close(); actions.onOpenFolder("Notebooks/$name") }) }
        }
    }
}

/** Nothing in the vault yet: a place to start, by hand or by asking HQ. */
@Composable
private fun EmptyNotes(actions: NotesActions, onAskHq: () -> Unit) {
    Column(Modifier.fillMaxSize().padding(horizontal = 32.dp), verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.size(72.dp).background(Mts.Surface200, RoundedCornerShape(20.dp)), contentAlignment = Alignment.Center) { Icon(HqIcons.Notes, null, Modifier.size(36.dp), tint = Mts.Ink) }
        Text(stringResource(R.string.notes_your_notes_will_show), style = HqType.H2, color = Mts.Ink)
        Text(stringResource(R.string.notes_write_one_yourself_or), style = HqType.BodySm, color = Mts.InkMuted)
        Box(Modifier.padding(top = 8.dp)) { NewNoteButton(stringResource(R.string.quick_note_title), Modifier) { actions.onNew(INBOX) } }
        Box(
            Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClick = onAskHq).padding(horizontal = 24.dp),
            contentAlignment = Alignment.Center,
        ) { Text(stringResource(R.string.notes_ask_hq_to_write), style = HqType.BodySmBold, color = Mts.Ink) }
    }
}
