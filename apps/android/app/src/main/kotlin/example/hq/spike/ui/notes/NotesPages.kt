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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
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
        Box(Modifier.fillMaxSize().background(Mts.Surface000).statusBarsPadding().navigationBarsPadding()) {
            when (top) {
                is NotesRoute.Reader -> ReaderPage(ui, actions)
                is NotesRoute.Folder -> FolderPage(ui, actions)
                is NotesRoute.Search -> SearchPage(ui, actions, onAskHq)
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
        IconAction(HqIcons.ArrowBack, "Back", iconSize = 24.dp, onClick = onBack)
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
private fun ReaderPage(ui: NotesUi, actions: NotesActions) {
    when (val phase = ui.reader) {
        is ReaderPhase.Ready -> ReaderReady(phase, actions)
        ReaderPhase.Loading -> Column { TopBar("Note", null, actions.onBack); Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Spinner(32) } }
        ReaderPhase.Gone -> GonePage(ui, actions)
        ReaderPhase.Failed -> Column {
            TopBar("Note", null, actions.onBack)
            Column(Modifier.fillMaxSize().padding(horizontal = 32.dp), verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally) {
                Text("HQ is not running", style = HqType.H2, color = Mts.Ink)
                Text("The note will open when HQ is back.", style = HqType.BodySm, color = Mts.InkMuted)
            }
        }
    }
}

@Composable
private fun ReaderReady(phase: ReaderPhase.Ready, actions: NotesActions) {
    val note = phase.note
    val blocks = remember(note.body) { parseBlocks(NoteText.withoutFrontmatter(note.body)) }
    val handlers = remember(actions) { BlockActions(onOpenNote = actions.onOpenNote, onCopied = { actions.onNotice("Copied") }) }
    Column(Modifier.fillMaxSize()) {
        TopBar(note.title, listOf(note.trail, phase.edited).filter { it.isNotBlank() }.joinToString(" · "), actions.onBack) {
            GreenPill("Edit", Modifier.padding(end = 4.dp), onClick = actions.onEdit)
            IconAction(HqIcons.MoreVertical, "More", iconSize = 24.dp) { actions.onMenu(true) }
        }
        // Lazy, so a long note opens by drawing the screenful in view, not every block of it.
        SelectionContainer(Modifier.weight(1f)) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(start = 16.dp, top = 16.dp, end = 16.dp, bottom = 32.dp)) {
                itemsIndexed(blocks) { i, block -> Box(Modifier.padding(top = blockGap(blocks, i))) { BlockView(block, handlers) } }
                if (note.truncated) item { Text("This note is long. Only the start is shown.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(top = 12.dp)) }
            }
        }
    }
}

/** A pinned note, a chip or a link that points at a note that is not there any more. */
@Composable
private fun GonePage(ui: NotesUi, actions: NotesActions) {
    val path = (ui.top as? NotesRoute.Reader)?.path.orEmpty()
    Column(Modifier.fillMaxSize()) {
        TopBar("Note", null, actions.onBack)
        Column(Modifier.fillMaxSize().padding(horizontal = 32.dp), verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally) {
            Box(Modifier.size(72.dp).background(Mts.Surface200, RoundedCornerShape(20.dp)), contentAlignment = Alignment.Center) { Icon(HqIcons.Warning, null, Modifier.size(34.dp), tint = Mts.Amber) }
            Text("This note is not here any more", style = HqType.H2, color = Mts.Ink)
            Text("It may have been moved or deleted. ${path.removePrefix("Notebooks/")}", style = HqType.BodySm, color = Mts.InkMuted)
            Box(Modifier.padding(top = 8.dp)) {
                Row(
                    Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button) { actions.onBack(); actions.onOpenSearch() }.padding(horizontal = 24.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
                ) { Icon(HqIcons.Search, null, Modifier.size(20.dp), tint = Mts.OnGreen); Text("Search for it", style = HqType.BodySmBold, color = Mts.OnGreen) }
            }
            Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onBack).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) { Text("Go to Notes", style = HqType.BodySmBold, color = Mts.BrandBlue) }
        }
    }
}

// ---------------------------------------------------------------- folder

@Composable
private fun FolderPage(ui: NotesUi, actions: NotesActions) {
    val view = ui.folder ?: return
    val name = view.path.substringAfterLast('/')
    Column(Modifier.fillMaxSize()) {
        val parents = view.path.removePrefix("Notebooks/").substringBeforeLast('/', "")
        TopBar(name, if (parents.isEmpty()) "Notes" else "Notes › " + parents.replace("/", " › "), actions.onBack) {
            IconAction(HqIcons.Search, "Search notes", iconSize = 24.dp, onClick = actions.onOpenSearch)
        }
        if (ui.offline) OfflineBanner()
        Box(Modifier.weight(1f)) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 96.dp)) {
                item {
                    Row(Modifier.padding(horizontal = 16.dp, vertical = 6.dp).heightIn(min = 36.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(HqIcons.Sort, null, Modifier.size(18.dp), tint = Mts.Ink)
                        Text("A to Z", style = HqType.BodySmBold, color = Mts.Ink)
                    }
                }
                if (view.folders.isNotEmpty()) item { SubFolders(view.folders, actions.onOpenFolder) }
                if (view.loading) item { Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(28) } }
                items(view.notes, key = { it }) { path ->
                    example.hq.spike.ui.notes.NoteRow(example.hq.spike.data.HqClient.NoteItem(path, NoteRefs.title(path), "", "", false)) { actions.onOpenNote(path) }
                }
                if (!view.loading && view.notes.isEmpty() && view.folders.isEmpty()) item { Text("Nothing in this folder yet.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }
            }
            NewNoteButton("New note here", Modifier.align(Alignment.BottomEnd).padding(16.dp)) { actions.onNew(view.path) }
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

// ---------------------------------------------------------------- search

@Composable
private fun SearchPage(ui: NotesUi, actions: NotesActions, onAskHq: (List<Pair<String, String>>) -> Unit) {
    val s = ui.search
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    val hits = remember(s.hits, s.filter) { s.hits.filter { s.filter == null || folderOf(it.path) == s.filter } }
    Column(Modifier.fillMaxSize().imePadding()) {
        Row(Modifier.fillMaxWidth().padding(start = 4.dp, end = 12.dp, top = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            IconAction(HqIcons.ArrowBack, "Back", iconSize = 24.dp, onClick = actions.onBack)
            val shape = RoundedCornerShape(24.dp)
            Row(Modifier.weight(1f).heightIn(min = 48.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                BasicTextField(
                    value = s.query, onValueChange = actions.onQuery, singleLine = true, textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
                    modifier = Modifier.weight(1f).focusRequester(focus),
                    decorationBox = { inner -> Box { if (s.query.isEmpty()) Text("Search your notes", style = HqType.Body, color = Mts.InkMuted); inner() } },
                )
                if (s.query.isNotEmpty()) Box(Modifier.size(32.dp).clickable(role = Role.Button, onClickLabel = "Clear") { actions.onQuery("") }.semantics { contentDescription = "Clear" }, contentAlignment = Alignment.Center) { Icon(HqIcons.Close, null, Modifier.size(20.dp), tint = Mts.InkMuted) }
            }
        }
        if (s.hits.isNotEmpty()) FilterChips(s.hits, s.filter, actions.onFilter)
        Box(Modifier.weight(1f)) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 96.dp)) {
                items(hits, key = { it.path }) { hit -> HitRow(hit, s.query) { actions.onOpenNote(hit.path) } }
                if (s.query.isNotBlank() && !s.loading && hits.isEmpty()) item { Text("Nothing matches that.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }
                if (s.loading) item { Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(28) } }
            }
            if (hits.isNotEmpty()) {
                val n = hits.take(ASK_MAX).size
                Row(
                    Modifier.align(Alignment.BottomCenter).padding(start = 16.dp, end = 16.dp, bottom = 12.dp).fillMaxWidth().heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface000).border(1.dp, Mts.Line, CircleShape)
                        .clickable(role = Role.Button) { onAskHq(hits.take(ASK_MAX).map { it.path to it.title }) },
                    horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(HqIcons.Spark, null, Modifier.size(20.dp), tint = Mts.Ink)
                    Text(askWords(hits.size), style = HqType.BodySmBold, color = Mts.Ink)
                }
            }
        }
    }
}

private const val ASK_MAX = 10

/** The label on the Ask HQ pill: HQ is given at most ten notes, and the label says so when there are more hits than that. */
internal fun askWords(found: Int): String {
    val n = found.coerceAtMost(ASK_MAX)
    return "Ask HQ about " + when { n == 1 -> "this note"; found > n -> "the first $n notes"; else -> "these $n notes" }
}

private fun folderOf(path: String) = path.removePrefix("Notebooks/").substringBefore('/', "")

@Composable
private fun FilterChips(hits: List<SearchHit>, selected: String?, onFilter: (String?) -> Unit) {
    val groups = hits.groupingBy { folderOf(it.path) }.eachCount().filterKeys { it.isNotBlank() }
    Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Filter("All ${hits.size}", selected == null) { onFilter(null) }
        groups.forEach { (name, n) -> Filter("$name $n", selected == name) { onFilter(name) } }
    }
}

@Composable
private fun Filter(label: String, on: Boolean, onClick: () -> Unit) {
    Box(
        Modifier.heightIn(min = 36.dp).clip(CircleShape).then(if (on) Modifier.background(Mts.BrandGreen) else Modifier.border(1.dp, Mts.Line, CircleShape)).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 14.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = if (on) Mts.OnGreen else Mts.Ink, maxLines = 1) }
}

@Composable
private fun HitRow(hit: SearchHit, query: String, onClick: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }.padding(horizontal = 16.dp, vertical = 12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(hit.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            val folder = folderOf(hit.path).ifBlank { hit.notebook }
            if (folder.isNotBlank()) Text(folder, style = HqType.Caption.copy(lineHeight = 24.sp), color = Mts.InkMuted, modifier = Modifier.padding(start = 12.dp))
        }
        val trail = NoteRefs.folderTrail(hit.path)
        if (trail.isNotBlank()) Text(trail, style = SmallText, color = Mts.BrandBlue, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (hit.snippet.isNotBlank()) Text(marked(Plain.strip(hit.snippet), query), style = HqType.BodySm, color = Mts.InkMuted, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 2.dp))
    }
}

/** The snippet with each word of the search picked out: on a green wash, in white. */
internal fun marked(snippet: String, query: String): AnnotatedString = buildAnnotatedString {
    val words = query.split(' ').map { it.trim() }.filter { it.length >= 2 }.map { Regex(Regex.escape(it), RegexOption.IGNORE_CASE) }
    val spans = words.flatMap { it.findAll(snippet).map { m -> m.range } }.sortedBy { it.first }
    var at = 0
    for (r in spans) {
        if (r.first < at) continue
        append(snippet.substring(at, r.first))
        withStyle(SpanStyle(color = Mts.Ink, background = Color(0x3822FF37))) { append(snippet.substring(r.first, r.last + 1)) }
        at = r.last + 1
    }
    append(snippet.substring(at))
}

// ---------------------------------------------------------------- writing

@Composable
private fun WritePage(w: WriteState, folders: List<String>, actions: NotesActions) {
    BackHandler(onBack = actions.onBack)
    val focus = LocalFocusManager.current
    Column(Modifier.fillMaxSize().imePadding()) {
        TopBar(if (w.isNew) "New note" else w.title, null, actions.onBack) {
            if (!w.isNew) IconAction(HqIcons.Eye, if (w.preview) "Edit" else "Preview", iconSize = 24.dp, onClick = actions.onPreview)
            GreenPill("Save", Modifier.padding(end = 8.dp), enabled = w.status != SaveStatus.Saving, onClick = actions.onSave)
        }
        if (!w.isNew) StatusLine(w.status)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 8.dp, end = 16.dp)) {
            if (w.isNew) {
                BasicTextField(
                    value = w.title, onValueChange = actions.onTitle, textStyle = HqType.H2.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink), modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
                    decorationBox = { inner -> Box(contentAlignment = Alignment.CenterStart) { if (w.title.isEmpty()) Text("Title", style = HqType.H2, color = Mts.InkMuted); inner() } },
                )
                Row(
                    Modifier.padding(top = 8.dp, bottom = 12.dp).heightIn(min = 36.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button) { actions.onChooseFolder(true) }.padding(horizontal = 14.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(HqIcons.Folder, null, Modifier.size(18.dp), tint = Mts.Ink)
                    Text("Saving to ${w.folder.removePrefix("Notebooks/")}", style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1)
                    Icon(HqIcons.ChevronDown, null, Modifier.size(16.dp), tint = Mts.Ink)
                }
                Box(Modifier.fillMaxWidth().padding(bottom = 12.dp).drawBehind { drawLine(Mts.Hairline, Offset(0f, 0f), Offset(size.width, 0f), 1.dp.toPx()) })
            }
            if (w.preview) {
                val blocks = remember(w.text.text) { parseBlocks(w.text.text) }
                Blocks(blocks, remember { BlockActions() })
            } else {
                BasicTextField(
                    value = w.text, onValueChange = actions.onText, textStyle = HqType.Body.copy(color = Mts.Ink, lineHeight = 26.sp), cursorBrush = SolidColor(Mts.BrandGreen),
                    modifier = Modifier.fillMaxWidth().heightIn(min = 240.dp),
                    decorationBox = { inner -> Box { if (w.text.text.isEmpty()) Text("Start writing", style = HqType.Body.copy(lineHeight = 26.sp), color = Mts.InkMuted); inner() } },
                )
            }
        }
        if (!w.preview) FormatBar(w, actions) { focus.clearFocus() }
    }
    if (w.chooser) FolderChooser(w, folders, actions)
    if (w.leaving) LeaveDialog(actions)
}

@Composable
private fun StatusLine(status: SaveStatus) {
    val (text, color) = when (status) {
        SaveStatus.Saved -> "Saved" to Mts.InkMuted
        SaveStatus.Unsaved -> "Unsaved changes" to Mts.Amber
        SaveStatus.Saving -> "Saving…" to Mts.InkMuted
        SaveStatus.Failed -> "Could not save. Try again." to Mts.DangerText
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
            Key("Heading", { edit(example.hq.spike.ui.notes.MarkdownEditing::heading) }) { Text("H", style = HqType.BodyBold.copy(fontSize = 18.sp), color = Mts.Ink) }
            Key("Bold", { edit { example.hq.spike.ui.notes.MarkdownEditing.wrap(it, "**") } }) { Text("B", style = HqType.BodyBold.copy(fontSize = 18.sp), color = Mts.Ink) }
            Key("Italic", { edit { example.hq.spike.ui.notes.MarkdownEditing.wrap(it, "*") } }) { Text("I", style = HqType.BodyBold.copy(fontSize = 18.sp, fontStyle = FontStyle.Italic, fontWeight = FontWeight.Normal), color = Mts.Ink) }
            Key("Bulleted list", { edit { example.hq.spike.ui.notes.MarkdownEditing.prefixLines(it, "- ") } }) { Icon(HqIcons.List, null, Modifier.size(22.dp), tint = Mts.Ink) }
            Key("Checklist", { edit { example.hq.spike.ui.notes.MarkdownEditing.prefixLines(it, "- [ ] ") } }) { Icon(HqIcons.Tasks, null, Modifier.size(22.dp), tint = Mts.Ink) }
            Key("Link", { edit(example.hq.spike.ui.notes.MarkdownEditing::link) }) { Icon(HqIcons.Link, null, Modifier.size(22.dp), tint = Mts.Ink) }
        }
        Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onDone).padding(horizontal = 16.dp), contentAlignment = Alignment.Center) { Text("Done", style = HqType.BodySmBold, color = Mts.BrandBlue) }
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
        Text("Save to", style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp))
        val shape = RoundedCornerShape(24.dp)
        BasicTextField(
            value = find, onValueChange = { find = it }, singleLine = true, textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp).fillMaxWidth().heightIn(min = 48.dp).clip(shape).background(Mts.Surface000).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp, vertical = 11.dp),
            decorationBox = { inner -> Box { if (find.isEmpty()) Text("Find a folder", style = HqType.Body, color = Mts.InkMuted); inner() } },
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
                        val sub = if (folder == INBOX) "Where new notes go unless you pick another" else NoteRefs.folderTrail("$folder/x").substringBeforeLast(" › ", "")
                        if (sub.isNotBlank()) Text(sub, style = HqType.BodySm, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    if (selected) Icon(HqIcons.Check, null, Modifier.size(22.dp), tint = Mts.BrandGreen)
                }
            }
        }
    }
}

@Composable
private fun LeaveDialog(actions: NotesActions) {
    BackHandler(onBack = actions.onKeepEditing)
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.62f)).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = actions.onKeepEditing))
        Column(Modifier.padding(start = 24.dp, top = 240.dp, end = 24.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Save your changes?", style = HqType.H2, color = Mts.Ink)
            Text("You have not saved what you wrote. Your text is kept on this phone either way.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(bottom = 12.dp))
            GreenPill("Save", Modifier.fillMaxWidth(), onClick = actions.onSave)
            Box(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClick = actions.onKeepEditing), contentAlignment = Alignment.Center) { Text("Keep editing", style = HqType.BodySmBold, color = Mts.Ink) }
            Box(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onDiscard), contentAlignment = Alignment.Center) { Text("Discard changes", style = HqType.BodySmBold, color = Mts.DangerText) }
        }
    }
}
