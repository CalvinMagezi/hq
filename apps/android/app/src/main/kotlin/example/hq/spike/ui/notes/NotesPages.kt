package example.hq.spike.ui.notes

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.runtime.DisposableEffect
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.ui.res.stringResource
import example.hq.spike.R
import example.hq.spike.ui.shell.ConfirmCopy
import example.hq.spike.ui.shell.ConfirmDialog
import example.hq.spike.ui.chat.blocks.Block
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.chat.ExportActions
import example.hq.spike.ui.chat.ExportOverlay
import example.hq.spike.ui.chat.NoteMenuActions
import example.hq.spike.ui.chat.NoteMenuSheet
import example.hq.spike.ui.chat.NoteSheet
import example.hq.spike.ui.chat.SheetFrame
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.chat.blocks.BlockActions
import example.hq.spike.ui.chat.blocks.Blocks
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.chat.blocks.NoteRefs
import example.hq.spike.ui.chat.blocks.NoteText
import androidx.compose.foundation.lazy.itemsIndexed
import example.hq.spike.ui.chat.blocks.BlockView
import example.hq.spike.ui.chat.blocks.blockGap
import example.hq.spike.ui.chat.blocks.Plain
import example.hq.spike.ui.chat.blocks.parseBlocks
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.consumeTouches
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.delay

private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private const val NOTICE_MS = 4_000L

/** The pages pushed over Notes, one at a time, with the note menu, the export steps and the line at the bottom. */
@Composable
fun NotesOverlays(ui: NotesUi, actions: NotesActions, onAskHq: (List<Pair<String, String>>) -> Unit) {
    val top = ui.top
    if (top != null) {
        BackHandler(onBack = actions.onBack)
        Box(Modifier.fillMaxSize().background(Mts.Surface000).consumeTouches().statusBarsPadding().navigationBarsPadding()) {
            when (top) {
                is NotesRoute.Reader -> ReaderPage(ui, actions, onAskHq)
                is NotesRoute.Folder -> FolderPage(ui, actions)
                is NotesRoute.Write -> ui.write?.let { WritePage(it, ui.folders, actions) }
            }
        }
    }
    val ready = ui.reader as? ReaderPhase.Ready
    if (ui.menu && ready != null && top is NotesRoute.Reader) {
        NoteMenuSheet(
            ready.note,
            NoteMenuActions(
                onClose = { actions.onMenu(false) }, onEdit = actions.onEdit, onTogglePin = actions.onTogglePin,
                onAsk = { onAskHq(listOf(ready.note.path to ready.note.title)) }, onExport = actions.onExport, onNotice = actions.onNotice,
            ),
        )
    }
    ui.export?.let { ExportOverlay(it, ExportActions(actions.onExportFormat, actions.onExportClose, actions.onExportAgain, actions.onNotice)) }
    ui.notice?.let { NotesSnack(it.text, actions.onDismissNotice, Modifier.fillMaxSize()) }
}

@Composable
private fun NotesSnack(text: String, onDismiss: () -> Unit, modifier: Modifier) {
    LaunchedEffect(text) { delay(NOTICE_MS); onDismiss() }
    Box(modifier, contentAlignment = Alignment.BottomCenter) {
        Row(
            Modifier.navigationBarsPadding().padding(horizontal = 8.dp, vertical = 8.dp).fillMaxWidth().heightIn(min = 48.dp).background(Mts.Ink, RoundedCornerShape(6.dp)).padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) { Text(text, style = HqType.BodySm, color = Mts.Surface100) }
    }
}

/** Back, a title with a line of context under it, and whatever the page puts at the end. */
@Composable
private fun TopBar(title: String, crumb: String?, onBack: () -> Unit, trailing: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 4.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        IconAction(HqIcons.ArrowBack, stringResource(R.string.notify_back), iconSize = 24.dp, onClick = onBack)
        Column(Modifier.weight(1f).padding(start = 4.dp)) {
            Text(title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (!crumb.isNullOrBlank()) Text(crumb, style = HqType.Caption, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        trailing()
    }
}

@Composable
private fun GreenPill(label: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    Box(
        modifier.heightIn(min = 40.dp).clip(CircleShape).background(if (enabled) Mts.BrandGreen else Mts.Surface200).clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 20.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = if (enabled) Mts.OnGreen else Mts.InkMuted) }
}

// ---------------------------------------------------------------- reading

@Composable
private fun ReaderPage(ui: NotesUi, actions: NotesActions, onAskHq: (List<Pair<String, String>>) -> Unit) {
    when (val phase = ui.reader) {
        is ReaderPhase.Ready -> ReaderReady(phase, actions) { onAskHq(listOf(phase.note.path to phase.note.title)) }
        ReaderPhase.Loading -> Column { TopBar(stringResource(R.string.quick_note_body), null, actions.onBack); Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Spinner(32) } }
        ReaderPhase.Gone -> GonePage(ui, actions)
        ReaderPhase.Failed -> Column {
            TopBar(stringResource(R.string.quick_note_body), null, actions.onBack)
            Column(Modifier.fillMaxSize().padding(horizontal = 32.dp), verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally) {
                Text(stringResource(R.string.notes_hq_is_not_running_2), style = HqType.H2, color = Mts.Ink)
                Text(stringResource(R.string.notes_the_note_will_open), style = HqType.BodySm, color = Mts.InkMuted)
            }
        }
    }
}

@Composable
private fun ReaderReady(phase: ReaderPhase.Ready, actions: NotesActions, onAsk: () -> Unit) {
    val note = phase.note
    val blocks = remember(note.body) { parseBlocks(NoteText.withoutFrontmatter(note.body)) }
    val copied = stringResource(R.string.chat_copied)
    val handlers = remember(actions, copied) { BlockActions(onOpenNote = actions.onOpenNote, onCopied = { actions.onNotice(copied) }) }
    Column(Modifier.fillMaxSize()) {
        TopBar(note.title, listOf(note.trail, phase.edited).filter { it.isNotBlank() }.joinToString(" · "), actions.onBack) {
            GreenPill(stringResource(R.string.note_editor_edit), Modifier.padding(end = 4.dp), onClick = actions.onEdit)
            IconAction(HqIcons.MoreVertical, stringResource(R.string.tasks_more), iconSize = 24.dp) { actions.onMenu(true) }
        }
        ReaderChips(note, actions, onAsk)
        // Lazy, so a long note opens by drawing the screenful in view, not every block of it.
        SelectionContainer(Modifier.weight(1f)) { NoteBody(note.path, blocks, handlers, note.truncated, actions) }
    }
}

/** What can be done with the note it is showing, under the bar. The row scrolls, so a narrow phone cuts the last chip. */
@Composable
private fun ReaderChips(note: example.hq.spike.ui.chat.NoteSheet, actions: NotesActions, onAsk: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val tooLong = stringResource(R.string.notes_too_long_share)
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        ReaderChip(HqIcons.Spark, stringResource(R.string.notes_ask_hq), Mts.BrandBlue, onAsk)
        ReaderChip(HqIcons.Tasks, stringResource(R.string.notes_make_task), Mts.Ink, actions.onMakeTask)
        ReaderChip(HqIcons.Pin, stringResource(if (note.pinned) R.string.pin_unpin else R.string.notes_pin), Mts.Ink, actions.onTogglePin)
        ReaderChip(HqIcons.Share, stringResource(R.string.notes_share), Mts.Ink) { if (note.truncated) actions.onNotice(tooLong) else example.hq.spike.ui.chat.shareText(context, note) }
    }
}

/** Drawn 40 high as the board has it; the touch target around it is 48. */
@Composable
private fun ReaderChip(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, tint: Color, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onClick), contentAlignment = Alignment.Center) {
        Row(
            Modifier.height(40.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).padding(horizontal = 14.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, null, Modifier.size(18.dp), tint = tint)
            Text(label, style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1)
        }
    }
}

private val ReaderPadding = androidx.compose.foundation.layout.PaddingValues(start = 16.dp, top = 16.dp, end = 16.dp, bottom = 32.dp)

@Composable
private fun NoteBody(path: String, blocks: List<Block>, handlers: BlockActions, truncated: Boolean, actions: NotesActions) {
    val spot = remember(path) { actions.scrollOf(path) }
    val list = rememberLazyListState(spot?.first ?: 0, spot?.second ?: 0)
    DisposableEffect(path) { onDispose { actions.onScrolled(path, list.firstVisibleItemIndex, list.firstVisibleItemScrollOffset) } }
    LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = ReaderPadding) {
        itemsIndexed(blocks) { i, block -> Box(Modifier.padding(top = blockGap(blocks, i))) { BlockView(block, handlers) } }
        if (truncated) item { Text(stringResource(R.string.notes_this_note_is_long), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(top = 12.dp)) }
    }
}

/** A pinned note, a chip or a link that points at a note that is not there any more. */
@Composable
private fun GonePage(ui: NotesUi, actions: NotesActions) {
    val path = (ui.top as? NotesRoute.Reader)?.path.orEmpty()
    val openSearch = example.hq.spike.ui.shell.LocalOpenSearch.current
    Column(Modifier.fillMaxSize()) {
        TopBar(stringResource(R.string.quick_note_body), null, actions.onBack)
        Column(Modifier.fillMaxSize().padding(horizontal = 32.dp), verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally) {
            Box(Modifier.size(72.dp).background(Mts.Surface200, RoundedCornerShape(20.dp)), contentAlignment = Alignment.Center) { Icon(HqIcons.Warning, null, Modifier.size(34.dp), tint = Mts.Amber) }
            Text(stringResource(R.string.notes_this_note_is_not), style = HqType.H2, color = Mts.Ink)
            val known = path != UNKNOWN_NOTE
            Text(if (known) stringResource(R.string.notes_may_be_moved, path.removePrefix("Notebooks/")) else stringResource(R.string.notes_gone_moved_or_deleted), style = HqType.BodySm, color = Mts.InkMuted)
            if (known) Box(Modifier.padding(top = 8.dp)) {
                Row(
                    Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button) { actions.onBack(); openSearch(example.hq.spike.ui.search.SearchStart(example.hq.spike.ui.search.SearchScope.Notes, null, example.hq.spike.ui.chat.blocks.NoteRefs.title(path))) }.padding(horizontal = 24.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
                ) { Icon(HqIcons.Search, null, Modifier.size(20.dp), tint = Mts.OnGreen); Text(stringResource(R.string.notes_search_for_it), style = HqType.BodySmBold, color = Mts.OnGreen) }
            }
            Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onBack).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) { Text(stringResource(R.string.notes_go_to_notes), style = HqType.BodySmBold, color = Mts.BrandBlue) }
        }
    }
}

// ---------------------------------------------------------------- folder

@Composable
private fun FolderPage(ui: NotesUi, actions: NotesActions) {
    val view = ui.folder ?: return
    val openSearch = example.hq.spike.ui.shell.LocalOpenSearch.current
    val name = view.path.substringAfterLast('/')
    Column(Modifier.fillMaxSize()) {
        val parents = view.path.removePrefix("Notebooks/").substringBeforeLast('/', "")
        TopBar(name, if (parents.isEmpty()) stringResource(R.string.place_notes) else stringResource(R.string.notes_crumb_in, parents.replace("/", " › ")), actions.onBack) {
            IconAction(HqIcons.Search, stringResource(R.string.search_field_notes), iconSize = 24.dp, onClick = { openSearch(example.hq.spike.ui.search.SearchStart(example.hq.spike.ui.search.SearchScope.Notes, view.path.removePrefix("Notebooks/"))) })
        }
        Box(Modifier.weight(1f)) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 96.dp)) {
                if (view.folders.isNotEmpty()) { item { SectionLabel(stringResource(R.string.notes_folders), view.folders.size) }; item { SubFolders(view.folders, actions.onOpenFolder) } }
                if (view.notes.isNotEmpty()) item { SectionLabel(stringResource(R.string.notes_a_to_z), view.notes.size) }
                if (view.loading) item { Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(28) } }
                items(view.notes, key = { it }) { path ->
                    example.hq.spike.ui.notes.NoteRow(example.hq.spike.data.HqClient.NoteItem(path, NoteRefs.title(path), "", "", false)) { actions.onOpenNote(path) }
                }
                if (!view.loading && view.notes.isEmpty() && view.folders.isEmpty()) item { Text(stringResource(R.string.notes_nothing_in_this_folder), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }
            }
            NewNoteButton(stringResource(R.string.notes_new_here), Modifier.align(Alignment.BottomEnd).padding(16.dp)) { actions.onNew(view.path) }
        }
    }
}

@Composable
private fun SubFolders(folders: List<Pair<String, Int>>, onOpen: (String) -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp, bottom = 8.dp).fillMaxWidth().clip(shape).background(Mts.Surface100)) {
        folders.forEachIndexed { i, (path, count) ->
            Row(
                Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { onOpen(path) }.drawBehind { if (i > 0) drawLine(Mts.Hairline, Offset(0f, 0f), Offset(size.width, 0f), 1.dp.toPx()) }.padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(HqIcons.Folder, null, Modifier.size(24.dp), tint = Mts.Ink)
                Text(path.substringAfterLast('/'), style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                Text("$count", style = HqType.BodySm, color = Mts.InkMuted)
                Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
        }
    }
}

// ---------------------------------------------------------------- writing

@Composable
private fun WritePage(w: WriteState, folders: List<String>, actions: NotesActions) {
    BackHandler(onBack = actions.onBack)
    val focus = LocalFocusManager.current
    Column(Modifier.fillMaxSize().imePadding()) {
        TopBar(if (w.isNew) stringResource(R.string.quick_note_title) else w.title, null, actions.onBack) {
            if (!w.isNew) IconAction(HqIcons.Eye, stringResource(if (w.preview) R.string.note_editor_edit else R.string.note_editor_preview), iconSize = 24.dp, onClick = actions.onPreview)
            GreenPill(stringResource(R.string.quick_note_save), Modifier.padding(end = 8.dp), enabled = w.status != SaveStatus.Saving, onClick = actions.onSave)
        }
        if (!w.isNew) StatusLine(w.status)
        w.conflict?.let { ChangedCard(it, actions) }
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 8.dp, end = 16.dp)) {
            if (w.isNew) {
                BasicTextField(
                    value = w.title, onValueChange = actions.onTitle, textStyle = HqType.H2.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink), modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
                    decorationBox = { inner -> Box(contentAlignment = Alignment.CenterStart) { if (w.title.isEmpty()) Text(stringResource(R.string.notes_title), style = HqType.H2, color = Mts.InkMuted); inner() } },
                )
                Row(
                    Modifier.padding(top = 8.dp, bottom = 12.dp).heightIn(min = 36.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button) { actions.onChooseFolder(true) }.padding(horizontal = 14.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(HqIcons.Folder, null, Modifier.size(18.dp), tint = Mts.Ink)
                    Text(stringResource(R.string.notes_saving_to, w.folder.removePrefix("Notebooks/")), style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1)
                    Icon(HqIcons.ChevronDown, null, Modifier.size(16.dp), tint = Mts.Ink)
                }
                Box(Modifier.fillMaxWidth().padding(bottom = 12.dp).drawBehind { drawLine(Mts.Hairline, Offset(0f, 0f), Offset(size.width, 0f), 1.dp.toPx()) })
            }
            if (w.preview) {
                val blocks = remember(w.text.text) { parseBlocks(w.text.text) }
                Blocks(blocks, remember { BlockActions() })
            } else {
                BasicTextField(
                    value = w.text, onValueChange = actions.onText, textStyle = HqType.Body.copy(color = Mts.Ink, lineHeight = 26.sp), cursorBrush = SolidColor(Mts.BrandGreen), visualTransformation = MarkdownTransformation,
                    modifier = Modifier.fillMaxWidth().heightIn(min = 240.dp),
                    decorationBox = { inner -> Box { if (w.text.text.isEmpty()) Text(stringResource(R.string.notes_start_writing), style = HqType.Body.copy(lineHeight = 26.sp), color = Mts.InkMuted); inner() } },
                )
            }
        }
        if (!w.preview) FormatBar(w, actions) { focus.clearFocus() }
    }
    if (w.chooser) FolderChooser(w, folders, actions)
    if (w.leaving) LeaveDialog(w.leaveName, actions)
}

/** The save HQ refused: the note changed elsewhere, so the owner picks what happens to their text. */
@Composable
private fun ChangedCard(c: Conflict, actions: NotesActions) {
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).border(1.dp, Mts.Amber, RoundedCornerShape(12.dp)).padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(HqIcons.Warning, null, Modifier.size(20.dp), tint = Mts.Amber)
            Text(stringResource(R.string.notes_changed_elsewhere), style = HqType.BodyBold, color = Mts.Ink)
        }
        c.by?.let { Text(stringResource(R.string.notes_changed_by, it), style = HqType.BodySm, color = Mts.InkMuted) }
        c.theirs?.let { Text(it.ifBlank { stringResource(R.string.notes_their_version_empty) }, style = HqType.BodySm, color = Mts.Ink, maxLines = 12, overflow = TextOverflow.Ellipsis) }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(enabled = !c.busy, role = Role.Button, onClick = actions.onSeeTheirs).padding(horizontal = 16.dp), contentAlignment = Alignment.Center) { Text(stringResource(R.string.notes_see_theirs), style = HqType.BodySmBold, color = Mts.Ink) }
            Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Ink).clickable(enabled = !c.busy, role = Role.Button, onClick = actions.onKeepBoth).padding(horizontal = 16.dp), contentAlignment = Alignment.Center) { Text(stringResource(R.string.notes_keep_both), style = HqType.BodySmBold, color = Mts.Surface000) }
        }
    }
}

@Composable
private fun StatusLine(status: SaveStatus) {
    val (text, color) = when (status) {
        SaveStatus.Saved -> stringResource(R.string.note_save_saved) to Mts.InkMuted
        SaveStatus.Unsaved -> stringResource(R.string.note_save_unsaved) to Mts.Amber
        SaveStatus.Saving -> stringResource(R.string.note_save_saving) to Mts.InkMuted
        SaveStatus.Failed -> stringResource(R.string.notes_save_failed) to Mts.DangerText
    }
    Row(Modifier.padding(start = 20.dp, bottom = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        if (status == SaveStatus.Unsaved) Box(Modifier.size(8.dp).background(Mts.Amber, CircleShape))
        Text(text, style = HqType.Caption, color = color)
    }
}

@Composable
private fun FormatBar(w: WriteState, actions: NotesActions, onDone: () -> Unit) {
    fun edit(change: (androidx.compose.ui.text.input.TextFieldValue) -> androidx.compose.ui.text.input.TextFieldValue) = actions.onText(change(w.text))
    Row(Modifier.fillMaxWidth().background(Mts.Surface100).drawBehind { drawLine(Mts.Hairline, Offset.Zero, Offset(size.width, 0f), 1.dp.toPx()) }, verticalAlignment = Alignment.CenterVertically) {
        Row(Modifier.weight(1f).horizontalScroll(rememberScrollState())) {
            Key(stringResource(R.string.notes_fmt_heading), { edit(example.hq.spike.ui.notes.MarkdownEditing::heading) }) { Text("H", style = HqType.BodyBold.copy(fontSize = 18.sp), color = Mts.Ink) }
            Key(stringResource(R.string.notes_fmt_bold), { edit { example.hq.spike.ui.notes.MarkdownEditing.wrap(it, "**") } }) { Text("B", style = HqType.BodyBold.copy(fontSize = 18.sp), color = Mts.Ink) }
            Key(stringResource(R.string.notes_fmt_italic), { edit { example.hq.spike.ui.notes.MarkdownEditing.wrap(it, "*") } }) { Text("I", style = HqType.BodyBold.copy(fontSize = 18.sp, fontStyle = FontStyle.Italic, fontWeight = FontWeight.Normal), color = Mts.Ink) }
            Key(stringResource(R.string.notes_fmt_bullets), { edit { example.hq.spike.ui.notes.MarkdownEditing.prefixLines(it, "- ") } }) { Icon(HqIcons.List, null, Modifier.size(22.dp), tint = Mts.Ink) }
            Key(stringResource(R.string.notes_fmt_checklist), { edit { example.hq.spike.ui.notes.MarkdownEditing.prefixLines(it, "- [ ] ") } }) { Icon(HqIcons.Tasks, null, Modifier.size(22.dp), tint = Mts.Ink) }
            Key(stringResource(R.string.notes_fmt_link), { edit(example.hq.spike.ui.notes.MarkdownEditing::link) }) { Icon(HqIcons.Link, null, Modifier.size(22.dp), tint = Mts.Ink) }
        }
        Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onDone).padding(horizontal = 16.dp), contentAlignment = Alignment.Center) { Text(stringResource(R.string.safe_done_button), style = HqType.BodySmBold, color = Mts.BrandBlue) }
    }
}

@Composable
private fun Key(label: String, onClick: () -> Unit, content: @Composable () -> Unit) {
    Box(Modifier.size(48.dp).clickable(role = Role.Button, onClickLabel = label, onClick = onClick).semantics { contentDescription = label }, contentAlignment = Alignment.Center) { content() }
}

@Composable
private fun FolderChooser(w: WriteState, folders: List<String>, actions: NotesActions) {
    var find by remember { mutableStateOf("") }
    val all = (listOf(INBOX) + folders).distinct().let { if (w.folder in it) it else it + w.folder }
    val shown = all.filter { find.isBlank() || it.contains(find, ignoreCase = true) }
    SheetFrame({ actions.onChooseFolder(false) }, top = 250.dp) {
        Text(stringResource(R.string.notes_save_to), style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp))
        val shape = RoundedCornerShape(24.dp)
        BasicTextField(
            value = find, onValueChange = { find = it }, singleLine = true, textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp).fillMaxWidth().heightIn(min = 48.dp).clip(shape).background(Mts.Surface000).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp, vertical = 11.dp),
            decorationBox = { inner -> Box { if (find.isEmpty()) Text(stringResource(R.string.notes_find_a_folder), style = HqType.Body, color = Mts.InkMuted); inner() } },
        )
        LazyColumn(Modifier.fillMaxWidth().weight(1f)) {
            items(shown, key = { it }) { folder ->
                val selected = folder == w.folder
                val depth = folder.removePrefix("Notebooks/").count { it == '/' }
                Row(
                    Modifier.padding(start = 8.dp + (depth * 16).dp, end = 8.dp).fillMaxWidth().heightIn(min = 64.dp).clip(RoundedCornerShape(16.dp)).then(if (selected) Modifier.background(Mts.Surface200) else Modifier)
                        .clickable(role = Role.Button) { actions.onPickFolder(folder) }.padding(horizontal = 16.dp, vertical = 8.dp),
                    horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(HqIcons.Folder, null, Modifier.size(24.dp), tint = Mts.Ink)
                    Column(Modifier.weight(1f)) {
                        Text(folder.substringAfterLast('/'), style = HqType.BodyBold, color = Mts.Ink, maxLines = 1)
                        val sub = if (folder == INBOX) stringResource(R.string.notes_inbox_hint) else NoteRefs.folderTrail("$folder/x").substringBeforeLast(" › ", "")
                        if (sub.isNotBlank()) Text(sub, style = HqType.BodySm, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    if (selected) Icon(HqIcons.Check, null, Modifier.size(22.dp), tint = Mts.BrandGreen)
                }
            }
        }
    }
}

@Composable
private fun LeaveDialog(name: String, actions: NotesActions) {
    val context = androidx.compose.ui.platform.LocalContext.current
    ConfirmDialog(ConfirmCopy.leaveNote(context, name.ifBlank { context.getString(R.string.leave_note_fallback) }), onKeep = actions.onKeepEditing, onAct = actions.onDiscard, onExtra = actions.onSave)
}
