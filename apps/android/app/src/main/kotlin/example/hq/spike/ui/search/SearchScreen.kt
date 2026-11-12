package example.hq.spike.ui.search

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.R
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.ui.chat.ChatRow
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.chat.blocks.NoteRefs
import example.hq.spike.ui.chat.blocks.Plain
import example.hq.spike.ui.helpers.HelperCard
import example.hq.spike.ui.notes.SearchHit
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.consumeTouches
import example.hq.spike.ui.shell.rememberWords
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.delay

/** What a result does when chosen, and the two things the failure line and the Ask HQ pill can do. */
class SearchGoes(
    val onNote: (String) -> Unit,
    val onTask: (TaskItem) -> Unit,
    val onChat: (ChatRow) -> Unit,
    val onHelper: (HelperCard) -> Unit,
    val onAsk: (List<SearchHit>) -> Unit,
    val onStartHq: () -> Unit,
)


/** One search over notes, tasks, chats and helpers, opened from any magnifier or pill. */
@Composable
fun SearchScreen(ui: SearchUi, actions: SearchActions, goes: SearchGoes, stopped: Boolean) {
    BackHandler(onBack = actions.onClose)
    val w = rememberWords()
    val focus = remember { FocusRequester() }
    val list = rememberLazyListState()
    LaunchedEffect(Unit) { focus.requestFocus() }
    LaunchedEffect(list.firstVisibleItemIndex, list.firstVisibleItemScrollOffset) { actions.onScrolled(list.firstVisibleItemIndex, list.firstVisibleItemScrollOffset) }
    val searching = ui.query.isNotBlank()
    val askable = if (ui.scope == SearchScope.All || ui.scope == SearchScope.Notes) ui.notes.take(SearchWords.ASK_MAX) else emptyList()
    Column(Modifier.fillMaxSize().background(Mts.Surface000).consumeTouches().statusBarsPadding().imePadding()) {
        FieldRow(ui, actions, focus)
        ScopeChips(ui, actions)
        if (ui.scope == SearchScope.Notes) FilterChips(ui, actions)
        if (ui.scope == SearchScope.Notes && ui.folder != null) FolderFrom(ui, actions)
        if (searching && ui.scope == SearchScope.Notes) FolderChips(ui, actions)
        LazyColumn(Modifier.weight(1f), state = list, contentPadding = PaddingValues(bottom = 24.dp)) {
            if (!searching) idle(ui, actions) else results(ui, actions, goes, stopped)
        }
        SearchWords.ask(w, askable.size)?.let { AskPill(it) { actions.onPicked(ui.query); goes.onAsk(askable) } }
        if (ui.cleared) ClearedSnack(actions)
    }
}

@Composable
private fun FieldRow(ui: SearchUi, actions: SearchActions, focus: FocusRequester) {
    val w = rememberWords()
    Row(Modifier.fillMaxWidth().padding(start = 4.dp, end = 12.dp, top = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        IconAction(HqIcons.ArrowBack, w.s(R.string.search_back), iconSize = 24.dp, onClick = actions.onClose)
        val shape = RoundedCornerShape(24.dp)
        Row(Modifier.weight(1f).heightIn(min = 48.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            BasicTextField(
                value = ui.query, onValueChange = actions.onQuery, singleLine = true, textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
                modifier = Modifier.weight(1f).focusRequester(focus),
                decorationBox = { inner -> Box { if (ui.query.isEmpty()) Text(SearchWords.field(w, ui.scope), style = HqType.Body, color = Mts.InkMuted); inner() } },
            )
            if (ui.query.isNotEmpty()) Box(Modifier.size(48.dp).clickable(role = Role.Button, onClickLabel = w.s(R.string.search_clear)) { actions.onQuery("") }.semantics { contentDescription = w.s(R.string.search_clear) }, contentAlignment = Alignment.Center) {
                Icon(HqIcons.Close, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
        }
    }
}

/** The scope tabs: the chosen one is ink, never green. */
@Composable
private fun ScopeChips(ui: SearchUi, actions: SearchActions) {
    val w = rememberWords()
    Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        SearchScope.entries.forEach { s ->
            val on = ui.scope == s
            Box(
                Modifier.heightIn(min = 48.dp).clip(CircleShape).then(if (on) Modifier.background(Mts.Ink) else Modifier.border(1.dp, Mts.Line, CircleShape)).semantics { selected = on }.clickable(role = Role.Tab) { actions.onScope(s) }.padding(horizontal = 14.dp),
                contentAlignment = Alignment.Center,
            ) { Text(SearchWords.chip(w, ui, s), style = HqType.BodySmBold, color = if (on) Mts.Surface000 else Mts.Ink, maxLines = 1) }
        }
    }
}

/**
 * "Written by HQ" and the tags the phone knows: each narrows the search and each says whether it is on. With no word typed
 * they look off and a tap only explains why, because HQ cannot filter an empty search.
 */
@Composable
private fun FilterChips(ui: SearchUi, actions: SearchActions) {
    val w = rememberWords()
    val can = SearchMatch.canFilter(ui.query)
    var hint by remember { mutableStateOf(false) }
    LaunchedEffect(can) { if (can) hint = false }
    Column {
        Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 2.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(w.s(R.string.vault_filter_hq), ui.writtenByHq, can) { if (can) actions.onWrittenByHq(!ui.writtenByHq) else hint = true }
            ui.tags.forEach { t ->
                val on = ui.tag.equals(t, ignoreCase = true)
                FilterChip("#$t", on, can) { if (can) actions.onTag(if (on) null else t) else hint = true }
            }
        }
        if (hint && !can) Text(w.s(R.string.vault_filter_hint), style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
    }
}

@Composable
private fun FilterChip(label: String, on: Boolean, enabledLook: Boolean, onToggle: () -> Unit) {
    Box(
        Modifier.heightIn(min = 48.dp).clip(CircleShape).alpha(if (enabledLook) 1f else 0.5f)
            .then(if (on) Modifier.background(Mts.Surface200) else Modifier.border(1.dp, Mts.Line, CircleShape))
            .toggleable(value = on, role = Role.Checkbox, onValueChange = { onToggle() }).padding(horizontal = 14.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1) }
}

@Composable
private fun FolderFrom(ui: SearchUi, actions: SearchActions) {
    val w = rememberWords()
    val folder = ui.folder ?: return
    val trail = SearchWords.trail(folder)
    Column(Modifier.padding(horizontal = 16.dp)) {
        Row(
            Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClickLabel = w.s(R.string.search_folder_from_label, trail)) { actions.onFolder(null) }.padding(start = 14.dp, end = 10.dp).semantics { contentDescription = w.s(R.string.search_folder_from_label, trail) },
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(w.s(R.string.search_folder_from, trail), style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Icon(HqIcons.Close, null, Modifier.size(18.dp), tint = Mts.InkMuted)
        }
        Text(w.s(R.string.search_folder_caption, trail), style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(vertical = 4.dp))
    }
}

@Composable
private fun FolderChips(ui: SearchUi, actions: SearchActions) {
    val w = rememberWords()
    val chips = SearchWords.folderChips(ui)
    if (chips.isEmpty()) return
    Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 2.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        chips.forEach { (folder, n) ->
            val on = ui.folder == folder
            Box(
                Modifier.heightIn(min = 48.dp).clip(CircleShape).then(if (on) Modifier.background(Mts.Surface200) else Modifier.border(1.dp, Mts.Line, CircleShape)).semantics { selected = on }.clickable(role = Role.Tab) { actions.onFolder(folder) }.padding(horizontal = 14.dp),
                contentAlignment = Alignment.Center,
            ) { Text(if (folder == null) w.s(R.string.search_folder_all, n) else w.s(R.string.search_folder_chip, SearchWords.trail(folder), n), style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1) }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.idle(ui: SearchUi, actions: SearchActions) {
    item(key = "hint") { val w = rememberWords(); Text(SearchWords.hint(w, ui.scope), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }
    if (ui.recent.isEmpty()) return
    item(key = "recent-head") {
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 12.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.search_recent), style = HqType.Group, color = Mts.InkMuted, modifier = Modifier.weight(1f))
            Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onClearRecent).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) { Text(stringResource(R.string.search_recent_clear), style = HqType.BodySmBold, color = Mts.BrandBlue) }
        }
    }
    ui.recent.forEach { q ->
        item(key = "recent-$q") {
            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button) { actions.onRecent(q) }.padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Icon(HqIcons.Search, null, Modifier.size(20.dp), tint = Mts.InkMuted)
                Text(q, style = HqType.Body, color = Mts.Ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.results(ui: SearchUi, actions: SearchActions, goes: SearchGoes, stopped: Boolean) {
    val all = ui.scope == SearchScope.All
    val cap = if (all) SearchWords.PREVIEW_ROWS else Int.MAX_VALUE
    fun pick(go: () -> Unit) { actions.onPicked(ui.query); go() }

    if (ui.notesFail != null && (all || ui.scope == SearchScope.Notes || ui.scope == SearchScope.Chats)) item(key = "notes-down") { Failure(ui.notesFail, stopped, goes.onStartHq) }
    if (ui.savedAtMs != null && (all || ui.scope == SearchScope.Tasks)) item(key = "saved") { SearchWords.savedLine(rememberWords(), ui)?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) } }
    if (ui.hqDown && ui.query.isNotBlank() && ui.scope == SearchScope.Chats) item(key = "wait") { Text(stringResource(R.string.search_notes_wait), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }

    if (all || ui.scope == SearchScope.Notes) {
        if (ui.notes.isNotEmpty() || ui.loading) item(key = "h-notes") { GroupHeader(SearchScope.Notes, ui.notes.size, ui.notes.size > cap) { actions.onScope(SearchScope.Notes) } }
        items(ui.notes.take(cap).size, key = { "n-${ui.notes[it].path}" }) { i -> NoteHitRow(ui.notes[i], ui.query) { pick { goes.onNote(ui.notes[i].path) } } }
        if (ui.loading) item(key = "loading") { Box(Modifier.fillMaxWidth().padding(20.dp), contentAlignment = Alignment.Center) { Spinner(28) } }
    }
    if ((all || ui.scope == SearchScope.Tasks) && ui.tasks.isNotEmpty()) {
        item(key = "h-tasks") { GroupHeader(SearchScope.Tasks, ui.tasks.size, ui.tasks.size > cap) { actions.onScope(SearchScope.Tasks) } }
        items(ui.tasks.take(cap).size, key = { "t-${ui.tasks[it].id}" }) { i -> TaskHitRow(ui.tasks[i]) { pick { goes.onTask(ui.tasks[i]) } } }
    }
    if ((all || ui.scope == SearchScope.Chats) && ui.chats.isNotEmpty()) {
        item(key = "h-chats") { GroupHeader(SearchScope.Chats, ui.chats.size, ui.chats.size > cap) { actions.onScope(SearchScope.Chats) } }
        items(ui.chats.take(cap).size, key = { "c-${ui.chats[it].id}" }) { i -> ChatHitRow(ui.chats[i]) { pick { goes.onChat(ui.chats[i]) } } }
    }
    if ((all || ui.scope == SearchScope.Helpers) && ui.helpers.isNotEmpty()) {
        item(key = "h-helpers") { GroupHeader(SearchScope.Helpers, ui.helpers.size, ui.helpers.size > cap) { actions.onScope(SearchScope.Helpers) } }
        items(ui.helpers.take(cap).size, key = { "h-${ui.helpers[it].id}" }) { i -> HelperHitRow(ui, ui.helpers[i]) { pick { goes.onHelper(ui.helpers[i]) } } }
    }
    if (SearchWords.noMatch(ui)) item(key = "none") { NoMatch(ui, actions) }
}

@Composable
private fun Failure(fail: NotesFail, stopped: Boolean, onStartHq: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Top) {
            Icon(HqIcons.Warning, null, Modifier.size(20.dp), tint = Mts.Amber)
            Text(stringResource(fail.line), style = HqType.BodySm, color = Mts.Amber)
        }
        if (stopped) Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClick = onStartHq).padding(horizontal = 16.dp), contentAlignment = Alignment.Center) {
            Text(stringResource(R.string.conn_start_hq), style = HqType.BodySmBold, color = Mts.Ink)
        }
    }
}

@Composable
private fun NoMatch(ui: SearchUi, actions: SearchActions) {
    val w = rememberWords()
    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(SearchWords.none(w, ui.scope), style = HqType.BodySm, color = Mts.InkMuted)
        if (ui.scope != SearchScope.All) Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button) { actions.onScope(SearchScope.All) }.padding(horizontal = 16.dp), contentAlignment = Alignment.CenterStart) {
            Text(w.s(R.string.search_everything), style = HqType.BodySmBold, color = Mts.Ink)
        }
    }
}

@Composable
private fun AskPill(label: String, onClick: () -> Unit) {
    Box(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), contentAlignment = Alignment.Center) {
        Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Ink).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 20.dp), contentAlignment = Alignment.Center) {
            Text(label, style = HqType.BodySmBold, color = Mts.Surface000, maxLines = 2)
        }
    }
}

/** "Recent searches cleared" with Undo, for 10 s or until the screen leaves. */
@Composable
private fun ClearedSnack(actions: SearchActions) {
    val text = stringResource(R.string.search_recent_cleared)
    val undo = stringResource(R.string.search_undo)
    example.hq.spike.ui.shell.AttentionOrInline(example.hq.spike.ui.shell.AttentionKind.UndoSnackbar, example.hq.spike.ui.shell.rememberAttentionKey(), Modifier, actions.onClearedShown) { m ->
        example.hq.spike.ui.shell.SnackbarRow(text, undo, actions.onUndoClear, m)
    }
}

@Composable
private fun GroupHeader(scope: SearchScope, count: Int, more: Boolean, onMore: () -> Unit) {
    val w = rememberWords()
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(SearchWords.scope(w, scope), style = HqType.Group, color = Mts.InkMuted)
        Box(Modifier.clip(CircleShape).background(Mts.Surface100).padding(horizontal = 8.dp, vertical = 2.dp)) { Text("$count", style = HqType.Badge, color = Mts.InkMuted) }
        Box(Modifier.weight(1f))
        if (more) Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onMore), contentAlignment = Alignment.Center) { Text(SearchWords.seeAll(w, count), style = HqType.BodySmBold, color = Mts.BrandBlue) }
    }
}

private fun Modifier.rowLine() = drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }

/** The text with each typed word in bold, on no background. */
internal fun bolded(text: String, query: String): AnnotatedString = buildAnnotatedString {
    val words = query.split(' ').map { it.trim() }.filter { it.length >= 2 }.map { Regex(Regex.escape(it), RegexOption.IGNORE_CASE) }
    val spans = words.flatMap { it.findAll(text).map { m -> m.range } }.sortedBy { it.first }
    var at = 0
    for (r in spans) {
        if (r.first < at) continue
        append(text.substring(at, r.first))
        withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(text.substring(r.first, r.last + 1)) }
        at = r.last + 1
    }
    append(text.substring(at))
}

@Composable
private fun NoteHitRow(hit: SearchHit, query: String, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).rowLine().padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(bolded(hit.title, query), style = HqType.BodyBold, color = Mts.Ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
        val trail = NoteRefs.folderTrail(hit.path)
        if (trail.isNotBlank()) Text(trail, style = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp), color = Mts.BrandBlue, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (hit.snippet.isNotBlank()) Text(bolded(Plain.oneLine(hit.snippet), query), style = HqType.BodySm, color = Mts.InkMuted, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 2.dp))
    }
}

@Composable
private fun TaskHitRow(task: TaskItem, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).rowLine().padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(task.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 3, overflow = TextOverflow.Ellipsis)
        Text(listOf(task.displayId, SearchMatch.statusWords(example.hq.spike.ui.shell.rememberWords(), task)).filter { it.isNotBlank() }.joinToString(" · "), style = HqType.BodySm, color = Mts.InkMuted, maxLines = 2)
    }
}

@Composable
private fun ChatHitRow(chat: ChatRow, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).rowLine().padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(chat.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
        if (chat.subtitle.isNotBlank()) Text(chat.subtitle, style = HqType.BodySm, color = Mts.InkMuted, maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun HelperHitRow(ui: SearchUi, h: HelperCard, onClick: () -> Unit) {
    val w = rememberWords()
    Column(Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).rowLine().padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(bolded(h.title, ui.query), style = HqType.BodyBold, color = Mts.Ink, maxLines = 3, overflow = TextOverflow.Ellipsis)
        Text(SearchWords.helperLine(w, ui, h), style = HqType.BodySm, color = Mts.InkMuted, maxLines = 3)
    }
}
