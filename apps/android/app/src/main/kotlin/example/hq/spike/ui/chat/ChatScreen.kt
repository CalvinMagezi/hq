package example.hq.spike.ui.chat

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.ui.chat.blocks.BlockActions
import example.hq.spike.ui.chat.blocks.ChipState
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.LocalGoTo
import example.hq.spike.ui.shell.Place
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.delay

/** The Chat v2 board of the design: chat name row, the conversation or the empty start, and the composer. */
@Composable
fun ChatScreen(state: ChatUiState, actions: ChatActions) {
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            if (!state.connected) ReconnectBanner()
            NameRow(state.title, actions.onOpenChats)
            Box(Modifier.weight(1f).fillMaxWidth()) {
                if (state.messages.isEmpty() && state.live == null) EmptyChat(actions.onDraft) else Conversation(state, actions)
            }
            Composer(state, actions)
        }
    }
}

/** Chip and tap behaviour for rich blocks, built from the chat state. */
@Composable
private fun rememberBlockActions(state: ChatUiState, actions: ChatActions): BlockActions {
    val go = LocalGoTo.current
    return remember(state.openingNote, state.missingNotes, actions) {
        BlockActions(
            onOpenNote = actions.onOpenNote, onOpenChart = actions.onOpenChart, onOpenTable = actions.onOpenTable,
            onOpenTasks = { go(Place.Tasks) }, onCopied = { actions.onNotice("Copied") }, onOpenFile = actions.onOpenFile,
            chipState = { path ->
                when {
                    path in state.missingNotes -> ChipState.Missing
                    path == state.openingNote -> ChipState.Opening
                    else -> ChipState.Ready
                }
            },
        )
    }
}

/** What sits over the whole shell, bars included: pages, the note sheet, the chats sheet and the line at the bottom. */
@Composable
fun ChatOverlays(state: ChatUiState, actions: ChatActions) {
    val go = LocalGoTo.current
    val blocks = rememberBlockActions(state, actions)
    Box(Modifier.fillMaxSize()) {
        when (val page = state.page) {
            is Page.Chart -> ChartPage(page.spec, actions.onClosePage)
            is Page.Table -> TablePage(page, actions.onClosePage) { actions.onNotice("Copied the table") }
            null -> {}
        }
        state.note?.let { sheet ->
            NoteSheetView(
                sheet, blocks, actions.onCloseNote, onOpenInNotes = { actions.onCloseNote(); go(Place.Notes) },
                onAsk = { actions.onAskAboutNote(sheet.path) }, onCopied = { actions.onNotice("Copied") },
            )
        }
        if (state.chatsOpen) ChatsSheet(state, actions)
        state.notice?.let { Snack(it, actions, Modifier.align(Alignment.BottomCenter)) }
    }
}

@Composable
private fun ReconnectBanner() {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).background(Mts.Surface100).drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }
            .padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.WifiOff, null, Modifier.size(20.dp), tint = Mts.Amber)
        Text("Reconnecting. Your draft is kept and can be sent once HQ is back.", style = HqType.BodySm, color = Mts.Ink)
    }
}

@Composable
private fun NameRow(title: String, onClick: () -> Unit) {
    val shape = RoundedCornerShape(24.dp)
    Row(
        Modifier.padding(horizontal = 16.dp).fillMaxWidth().heightIn(min = 48.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape)
            .clickable(role = Role.Button, onClickLabel = "Show your chats", onClick = onClick).padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.Chat, null, Modifier.size(20.dp), tint = Mts.InkMuted)
        Text(title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        Icon(HqIcons.ChevronDown, null, Modifier.size(18.dp), tint = Mts.InkMuted)
    }
}

@Composable
private fun Conversation(state: ChatUiState, actions: ChatActions) {
    // The list runs from the bottom, so the newest item is first and the failed card sits under a half-written reply.
    val newestFirst = state.messages.asReversed()
    val blocks = rememberBlockActions(state, actions)
    LazyColumn(
        Modifier.fillMaxSize(), reverseLayout = true, contentPadding = PaddingValues(start = 16.dp, top = 12.dp, end = 16.dp, bottom = 8.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.Bottom),
    ) {
        if (state.failed && state.live == null) item(key = "failed") { FailedCard(actions.onRetry) }
        state.live?.let { item(key = "live") { LiveReplyBubble(it, blocks) } }
        items(newestFirst, key = { it.id }) { m ->
            if (m.speaker == Speaker.User) UserBubble(m.text, m.sending, m.files, actions.onOpenFile) else ReplyBubble(m, blocks)
        }
    }
}

private val Suggestions = listOf(
    Triple(HqIcons.Spark, "Summarise this note", "Summarise this note"),
    Triple(HqIcons.Tasks, "What needs me today?", "What needs me today?"),
    Triple(HqIcons.Notes, "Save a thought as a note", "Save a thought as a note"),
)

@Composable
private fun EmptyChat(onPick: (String) -> Unit) {
    Column(Modifier.fillMaxSize().padding(horizontal = 16.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.Bottom)) {
        Image(painterResource(R.drawable.hq_mark), null, Modifier.size(56.dp).alpha(0.8f), contentScale = ContentScale.Fit)
        Column {
            Text("What would you like to do?", style = HqType.H2, color = Mts.Ink)
            Text("HQ can read your notes and tasks, write to them, and send helpers to do work.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(top = 4.dp))
        }
        Suggestions.forEach { (icon, label, text) -> SuggestionChip(icon, label) { onPick(text) } }
    }
}

@Composable
private fun SuggestionChip(icon: ImageVector, label: String, onClick: () -> Unit) {
    val shape = RoundedCornerShape(16.dp)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape)
            .clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(22.dp), tint = Mts.BrandBlue)
        Text(label, style = HqType.ChipTitle, color = Mts.Ink)
    }
}

/** One line at the bottom: what just happened, and sometimes a word to undo it. Gone after five seconds. */
@Composable
private fun Snack(notice: Notice, actions: ChatActions, modifier: Modifier) {
    LaunchedEffect(notice) {
        delay(NOTICE_MS)
        actions.onDismissNotice()
    }
    Row(
        modifier.navigationBarsPadding().padding(horizontal = 8.dp, vertical = 8.dp).fillMaxWidth().heightIn(min = 48.dp).background(Mts.Ink, RoundedCornerShape(6.dp)).padding(start = 16.dp),
        horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(notice.text, style = HqType.BodySm, color = Mts.Surface100, modifier = Modifier.weight(1f))
        notice.action?.let {
            Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onUndo).padding(horizontal = 16.dp), contentAlignment = Alignment.Center) {
                Text(it, style = HqType.BodySmBold, color = Mts.UndoBlue)
            }
        }
    }
}

private const val NOTICE_MS = 5_000L
