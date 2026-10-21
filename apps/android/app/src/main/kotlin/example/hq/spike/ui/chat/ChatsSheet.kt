package example.hq.spike.ui.chat

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.runtime.remember
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val SheetShape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
private val SheetTop = 152.dp
private val ScrimColor = Color.Black.copy(alpha = 0.62f)
private val ArchivePanel = Mts.BrandIndigo

/**
 * The chats sheet: the design's own overlay, with its top edge 152 dp down so the chat name row stays visible above it.
 * Tapping the dimmed part or pressing Back closes it.
 */
@Composable
fun ChatsSheet(state: ChatUiState, actions: ChatActions) {
    BackHandler(onBack = actions.onCloseChats)
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(ScrimColor).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = actions.onCloseChats))
        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(top = SheetTop).fillMaxHeight().clip(SheetShape).background(Mts.Surface100).navigationBarsPadding()) {
            Box(Modifier.padding(top = 12.dp, bottom = 12.dp).size(width = 32.dp, height = 4.dp).align(Alignment.CenterHorizontally).background(Mts.Line, RoundedCornerShape(2.dp)))
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("Chats", style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                NewChatButton(actions.onNewChat)
            }
            SearchField(state.chatSearch, actions.onSearchChats)
            val shown = state.chats.filter { it.title.contains(state.chatSearch, true) || it.subtitle.contains(state.chatSearch, true) }
            LazyColumn(Modifier.fillMaxWidth()) {
                Group.entries.forEach { group ->
                    val rows = shown.filter { it.group == group }
                    if (rows.isNotEmpty()) {
                        item(key = "g-${group.name}") { GroupHeader(group.label) }
                        items(rows, key = { it.id }) { ChatRowItem(it, selected = it.id == state.threadId, actions) }
                    }
                }
            }
        }
    }
}

@Composable
private fun NewChatButton(onClick: () -> Unit) {
    Row(
        Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 24.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.Plus, null, Modifier.size(20.dp), tint = Mts.OnGreen)
        Text("New chat", style = HqType.BodySmBold, color = Mts.OnGreen)
    }
}

@Composable
private fun SearchField(query: String, onChange: (String) -> Unit) {
    Row(
        Modifier.padding(start = 16.dp, end = 16.dp, bottom = 4.dp).fillMaxWidth().heightIn(min = 48.dp).clip(RoundedCornerShape(24.dp))
            .background(Mts.Surface000).border(1.dp, Mts.Line, RoundedCornerShape(24.dp)).padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.Search, null, Modifier.size(20.dp), tint = Mts.InkMuted)
        BasicTextField(
            query, onChange, Modifier.weight(1f), singleLine = true, textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
            decorationBox = { inner -> Box { if (query.isEmpty()) Text("Search chats", style = HqType.Body, color = Mts.InkMuted); inner() } },
        )
    }
}

@Composable
private fun GroupHeader(label: String) {
    Text(label.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 14.dp, end = 16.dp, bottom = 4.dp))
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ChatRowItem(row: ChatRow, selected: Boolean, actions: ChatActions) {
    val swipe = rememberSwipeToDismissBoxState(confirmValueChange = {
        (it == SwipeToDismissBoxValue.EndToStart).also { archive -> if (archive) actions.onArchive(row.id) }
    })
    SwipeToDismissBox(
        swipe, enableDismissFromStartToEnd = false,
        backgroundContent = {
            // Only while the row is being dragged; drawn always it shows as a faint outline round the row.
            if (swipe.dismissDirection == SwipeToDismissBoxValue.EndToStart) {
                Box(Modifier.fillMaxSize().padding(horizontal = 8.dp).background(ArchivePanel, RoundedCornerShape(16.dp)), contentAlignment = Alignment.CenterEnd) {
                    Column(Modifier.width(88.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                        Icon(HqIcons.Archive, null, Modifier.size(24.dp), tint = Mts.Ink)
                        Text("Archive", style = HqType.NavLabel, color = Mts.Ink)
                    }
                }
            }
        },
    ) {
        Row(
            Modifier.padding(horizontal = 8.dp).fillMaxWidth().heightIn(min = 64.dp).clip(RoundedCornerShape(16.dp))
                .background(if (selected) Mts.Surface200 else Mts.Surface100).clickable(role = Role.Button) { actions.onOpenChat(row.id) }
                .padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.size(40.dp).background(if (selected) Mts.Surface100 else Mts.Surface200, CircleShape), contentAlignment = Alignment.Center) {
                if (row.running) Spinner(22, Mts.BrandGreen) else Icon(HqIcons.Chat, null, Modifier.size(22.dp), tint = Mts.Ink)
            }
            Column(Modifier.weight(1f)) {
                Text(row.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                val subtitle = if (row.running) "Working" else row.subtitle
                if (subtitle.isNotEmpty()) Text(subtitle, style = HqType.BodySm, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (row.unread) Box(Modifier.size(10.dp).background(Mts.BrandGreen, CircleShape))
        }
    }
}
