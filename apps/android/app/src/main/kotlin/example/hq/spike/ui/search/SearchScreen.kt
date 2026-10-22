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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.ui.chat.ChatRow
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.chat.blocks.NoteRefs
import example.hq.spike.ui.chat.blocks.Plain
import example.hq.spike.ui.notes.SearchHit
import example.hq.spike.ui.notes.marked
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** One search over notes, tasks and chats, opened from the magnifier on any screen. */
@Composable
fun SearchScreen(ui: SearchUi, actions: SearchActions, onNote: (String) -> Unit, onTask: (TaskItem) -> Unit, onChat: (ChatRow) -> Unit) {
    BackHandler(onBack = actions.onClose)
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    val searching = ui.query.isNotBlank()
    Column(Modifier.fillMaxSize().background(Mts.Surface000).statusBarsPadding().imePadding()) {
        FieldRow(ui, actions, focus)
        if (searching) ScopeChips(ui, actions)
        LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(bottom = 24.dp)) {
            when {
                !searching -> idle(ui, actions)
                else -> results(ui, actions, onNote, onTask, onChat)
            }
        }
    }
}

@Composable
private fun FieldRow(ui: SearchUi, actions: SearchActions, focus: FocusRequester) {
    Row(Modifier.fillMaxWidth().padding(start = 4.dp, end = 12.dp, top = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        IconAction(HqIcons.ArrowBack, "Back", iconSize = 24.dp, onClick = actions.onClose)
        val shape = RoundedCornerShape(24.dp)
        Row(Modifier.weight(1f).heightIn(min = 48.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            BasicTextField(
                value = ui.query, onValueChange = actions.onQuery, singleLine = true, textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
                modifier = Modifier.weight(1f).focusRequester(focus),
                decorationBox = { inner -> Box { if (ui.query.isEmpty()) Text(SearchWords.FIELD, style = HqType.Body, color = Mts.InkMuted); inner() } },
            )
            if (ui.query.isNotEmpty()) Box(Modifier.size(32.dp).clickable(role = Role.Button, onClickLabel = "Clear") { actions.onQuery("") }.semantics { contentDescription = "Clear" }, contentAlignment = Alignment.Center) {
                Icon(HqIcons.Close, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
        }
    }
}

@Composable
private fun ScopeChips(ui: SearchUi, actions: SearchActions) {
    Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        SearchScope.entries.forEach { s ->
            val on = ui.scope == s
            Box(
                Modifier.heightIn(min = 36.dp).clip(CircleShape).then(if (on) Modifier.background(Mts.BrandGreen) else Modifier.border(1.dp, Mts.Line, CircleShape)).clickable(role = Role.Button) { actions.onScope(s) }.padding(horizontal = 14.dp),
                contentAlignment = Alignment.Center,
            ) { Text(SearchWords.chip(s, SearchWords.countOf(ui, s), true), style = HqType.BodySmBold, color = if (on) Mts.OnGreen else Mts.Ink, maxLines = 1) }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.idle(ui: SearchUi, actions: SearchActions) {
    if (ui.recent.isEmpty()) {
        item { Text(SearchWords.EMPTY_HINT, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }
        return
    }
    item { Text(SearchWords.RECENT_TITLE, style = HqType.Group, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 4.dp)) }
    ui.recent.forEach { q ->
        item(key = "recent-$q") {
            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button) { actions.onRecent(q) }.padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Icon(HqIcons.Search, null, Modifier.size(20.dp), tint = Mts.InkMuted)
                Text(q, style = HqType.Body, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.results(ui: SearchUi, actions: SearchActions, onNote: (String) -> Unit, onTask: (TaskItem) -> Unit, onChat: (ChatRow) -> Unit) {
    val all = ui.scope == SearchScope.All
    val cap = if (all) SearchWords.PREVIEW_ROWS else Int.MAX_VALUE
    fun pick(go: () -> Unit) { actions.onPicked(ui.query); go() }

    if (all || ui.scope == SearchScope.Notes) {
        if (ui.notes.isNotEmpty() || ui.loading || ui.notesFailed) item(key = "h-notes") { GroupHeader("Notes", ui.notes.size, ui.notes.size > cap) { actions.onScope(SearchScope.Notes) } }
        items(ui.notes.take(cap).size, key = { "n-${ui.notes[it].path}" }) { i -> NoteHitRow(ui.notes[i], ui.query) { pick { onNote(ui.notes[i].path) } } }
        if (ui.loading) item(key = "loading") { Box(Modifier.fillMaxWidth().padding(20.dp), contentAlignment = Alignment.Center) { Spinner(28) } }
        if (ui.notesFailed) item(key = "notes-down") { Text(SearchWords.NOTES_DOWN, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) }
    }
    if ((all || ui.scope == SearchScope.Tasks) && ui.tasks.isNotEmpty()) {
        item(key = "h-tasks") { GroupHeader("Tasks", ui.tasks.size, ui.tasks.size > cap) { actions.onScope(SearchScope.Tasks) } }
        items(ui.tasks.take(cap).size, key = { "t-${ui.tasks[it].id}" }) { i -> TaskHitRow(ui.tasks[i]) { pick { onTask(ui.tasks[i]) } } }
    }
    if ((all || ui.scope == SearchScope.Chats) && ui.chats.isNotEmpty()) {
        item(key = "h-chats") { GroupHeader("Chats", ui.chats.size, ui.chats.size > cap) { actions.onScope(SearchScope.Chats) } }
        items(ui.chats.take(cap).size, key = { "c-${ui.chats[it].id}" }) { i -> ChatHitRow(ui.chats[i]) { pick { onChat(ui.chats[i]) } } }
    }
    if (SearchWords.noMatch(ui)) item(key = "none") { Text(SearchWords.NO_MATCH, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }
}

@Composable
private fun GroupHeader(label: String, count: Int, more: Boolean, onMore: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(label, style = HqType.Group, color = Mts.InkMuted)
        Box(Modifier.clip(CircleShape).background(Mts.Surface100).padding(horizontal = 8.dp, vertical = 2.dp)) { Text("$count", style = HqType.Badge, color = Mts.InkMuted) }
        Box(Modifier.weight(1f))
        if (more) Box(Modifier.heightIn(min = 36.dp).clickable(role = Role.Button, onClick = onMore), contentAlignment = Alignment.Center) { Text(SearchWords.seeAll(count), style = HqType.BodySmBold, color = Mts.BrandBlue) }
    }
}

private fun Modifier.rowLine() = drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }

@Composable
private fun NoteHitRow(hit: SearchHit, query: String, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).rowLine().padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(hit.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
        val trail = NoteRefs.folderTrail(hit.path)
        if (trail.isNotBlank()) Text(trail, style = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp), color = Mts.BrandBlue, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (hit.snippet.isNotBlank()) Text(marked(Plain.oneLine(hit.snippet), query), style = HqType.BodySm, color = Mts.InkMuted, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 2.dp))
    }
}

@Composable
private fun TaskHitRow(task: TaskItem, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).rowLine().padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(task.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
        Text(listOf(task.displayId, SearchMatch.statusWords(task)).filter { it.isNotBlank() }.joinToString(" · "), style = HqType.BodySm, color = Mts.InkMuted, maxLines = 1)
    }
}

@Composable
private fun ChatHitRow(chat: ChatRow, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onClick).rowLine().padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(chat.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (chat.subtitle.isNotBlank()) Text(chat.subtitle, style = HqType.BodySm, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
