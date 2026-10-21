package example.hq.spike.ui.notes

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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.data.HqClient.NoteItem
import example.hq.spike.ui.chat.blocks.NoteRefs
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
internal const val INBOX = "Notebooks/Inbox"

/** The Notes place: a search box, folders, pinned notes and the newest notes, with a button for a new one. */
@Composable
fun NotesHome(ui: NotesUi, actions: NotesActions, onAskHq: () -> Unit) {
    LaunchedEffect(Unit) { actions.onRefresh() }
    Box(Modifier.fillMaxSize()) {
        when {
            ui.loaded && ui.recent.isEmpty() && ui.pinned.isEmpty() && ui.folders.isEmpty() && !ui.offline -> EmptyNotes(actions, onAskHq)
            else -> NotesList(ui, actions)
        }
    }
}

@Composable
private fun NotesList(ui: NotesUi, actions: NotesActions) {
    Box(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
            if (ui.offline) item { OfflineBanner() }
            item { SearchPill(actions.onOpenSearch) }
            if (ui.topFolders.isNotEmpty()) item { FolderChips(ui.topFolders) { actions.onOpenFolder("Notebooks/$it") } }
            if (ui.pinned.isNotEmpty()) {
                item { SectionLabel("Pinned") }
                item {
                    LazyRow(contentPadding = PaddingValues(start = 16.dp, top = 4.dp, end = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        items(ui.pinned, key = { it.path }) { PinnedCard(it) { actions.onOpenNote(it.path) } }
                    }
                }
            }
            if (ui.recent.isNotEmpty()) item { SectionLabel("Recent") }
            items(ui.recent, key = { it.path }) { NoteRow(it) { actions.onOpenNote(it.path) } }
        }
        NewNoteButton("New note", Modifier.align(Alignment.BottomEnd).padding(16.dp)) { actions.onNew(INBOX) }
    }
}

@Composable
internal fun OfflineBanner() {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).background(Mts.Surface100).drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }.padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.WifiOff, null, Modifier.size(20.dp), tint = Mts.Amber)
        Text("HQ is not running. Your notes will show again when it starts.", style = HqType.BodySm, color = Mts.Ink)
    }
}

@Composable
private fun SearchPill(onClick: () -> Unit) {
    val shape = RoundedCornerShape(26.dp)
    Row(
        Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp).fillMaxWidth().heightIn(min = 52.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape)
            .clickable(role = Role.Button, onClickLabel = "Search notes", onClick = onClick).padding(horizontal = 18.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.Search, null, Modifier.size(22.dp), tint = Mts.InkMuted)
        Text("Search your notes", style = HqType.Body, color = Mts.InkMuted)
    }
}

@Composable
private fun FolderChips(names: List<String>, onOpen: (String) -> Unit) {
    Row(Modifier.horizontalScroll(rememberScrollState()).padding(start = 16.dp, top = 14.dp, end = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        names.forEach { name ->
            Row(
                Modifier.heightIn(min = 40.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClickLabel = "Open $name") { onOpen(name) }.padding(horizontal = 14.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(HqIcons.Folder, null, Modifier.size(18.dp), tint = Mts.Ink)
                Text(name, style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1)
            }
        }
    }
}

@Composable
internal fun SectionLabel(text: String) {
    Text(text.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 20.dp, end = 16.dp, bottom = 4.dp))
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
        if (trail.isNotBlank()) Text(trail, style = SmallText, color = Mts.BrandBlue, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
internal fun NewNoteButton(label: String, modifier: Modifier, onClick: () -> Unit) {
    Row(
        modifier.heightIn(min = 56.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button, onClick = onClick).padding(start = 20.dp, end = 24.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.Plus, null, Modifier.size(24.dp), tint = Mts.OnGreen)
        Text(label, style = HqType.BodySmBold, color = Mts.OnGreen)
    }
}

/** Nothing in the vault yet: a place to start, by hand or by asking HQ. */
@Composable
private fun EmptyNotes(actions: NotesActions, onAskHq: () -> Unit) {
    Column(Modifier.fillMaxSize().padding(horizontal = 32.dp), verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.size(72.dp).background(Mts.Surface200, RoundedCornerShape(20.dp)), contentAlignment = Alignment.Center) { Icon(HqIcons.Notes, null, Modifier.size(36.dp), tint = Mts.Ink) }
        Text("Your notes will show up here", style = HqType.H2, color = Mts.Ink)
        Text("Write one yourself, or ask HQ to write one for you.", style = HqType.BodySm, color = Mts.InkMuted)
        Box(Modifier.padding(top = 8.dp)) { NewNoteButton("New note", Modifier) { actions.onNew(INBOX) } }
        Box(
            Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClick = onAskHq).padding(horizontal = 24.dp),
            contentAlignment = Alignment.Center,
        ) { Text("Ask HQ to write one", style = HqType.BodySmBold, color = Mts.Ink) }
    }
}
