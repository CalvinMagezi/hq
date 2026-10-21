package example.hq.spike.ui.helpers

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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.inbox.PrimaryButton
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.delay

private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private const val LINE_MS = 4_000L

/** The Helpers place: filter chips, a card for each helper, finished ones folded into one row, and a button to start another. */
@Composable
fun HelpersScreen(ui: HelpersUi, actions: HelpersActions) {
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Chips(ui, actions)
            Feed(ui, actions)
        }
        StartButton(Modifier.align(Alignment.BottomEnd).padding(16.dp), actions.onStartOpen)
        ui.line?.let { Snack(it, actions.onDismissLine, Modifier.align(Alignment.BottomCenter).padding(bottom = 4.dp)) }
    }
}

@Composable
private fun Chips(ui: HelpersUi, actions: HelpersActions) {
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        HelperFilter.entries.forEach { f ->
            val on = f == ui.filter
            val count = if (f == HelperFilter.All) null else ui.count(f)
            val label = when (f) { HelperFilter.All -> "All"; HelperFilter.Needs -> "Needs your OK"; HelperFilter.Working -> "Working"; HelperFilter.Finished -> "Finished" }
            Box(
                Modifier.heightIn(min = 48.dp).clip(CircleShape).selectable(selected = on, role = Role.Tab, onClick = { actions.onFilter(f) }),
                contentAlignment = Alignment.Center,
            ) {
                Box(
                    Modifier.heightIn(min = 36.dp).clip(CircleShape).then(if (on) Modifier.background(Mts.BrandGreen, CircleShape).border(1.dp, Mts.BrandGreen, CircleShape) else Modifier.border(1.dp, Mts.Line, CircleShape)).padding(horizontal = 14.dp),
                    contentAlignment = Alignment.Center,
                ) { Text(if (count != null) "$label $count" else label, style = HqType.BodySmBold, color = if (on) Mts.OnGreen else Mts.Ink, maxLines = 1) }
            }
        }
    }
}

@Composable
private fun Feed(ui: HelpersUi, actions: HelpersActions) {
    val shown = ui.cards.filter { matches(it, ui.filter) }
    val live = shown.filter { it.state != HelperState.Finished && it.state != HelperState.Stopped }
    val past = shown.filter { it.state == HelperState.Finished || it.state == HelperState.Stopped }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, top = 4.dp, end = 16.dp, bottom = 104.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (ui.offline) item { Text("HQ is not running. These are the last helpers it had.", style = HqType.BodySm, color = Mts.Amber) }
        if (shown.isEmpty() && ui.loaded && !ui.offline) item { Text(emptyWords(ui.filter), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(vertical = 8.dp)) }
        items(live, key = { it.id }) { HelperCardView(it, actions) }
        if (past.isNotEmpty()) {
            val open = ui.finishedOpen || ui.filter == HelperFilter.Finished
            item(key = "finished") { FinishedRow(past, open) { actions.onToggleFinished() } }
            if (open) items(past, key = { "p-${it.id}" }) { PastRow(it) { actions.onOpen(it.id) } }
        }
    }
}

private fun emptyWords(f: HelperFilter) = when (f) {
    HelperFilter.All -> "No helpers yet. Start one and it shows up here."
    HelperFilter.Needs -> "Nothing is waiting for your OK."
    HelperFilter.Working -> "No helper is working right now."
    HelperFilter.Finished -> "No finished helpers."
}

@Composable
private fun HelperCardView(c: HelperCard, actions: HelpersActions) {
    val shape = RoundedCornerShape(16.dp)
    val waiting = c.state == HelperState.NeedsOk
    Column(
        Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, if (waiting) Mts.Amber else Mts.Hairline, shape)
            .clickable(role = Role.Button, onClickLabel = "Open helper") { actions.onOpen(c.id) }.padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Top) {
            Text(c.title, style = HqType.BodyBold.copy(fontSize = 17.sp), color = Mts.Ink, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            StatePill(c.state)
        }
        Text(listOf("${c.agent} · on ${c.where}", c.time).filter { it.isNotBlank() }.joinToString(" · "), style = HqType.BodySm, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(c.line, style = HqType.BodySm, color = Mts.Ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
        if (waiting && c.ask != null && HelperWords.canAnswerInline(c.ask)) {
            if (c.ask.kind == AskKind.Command) Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).background(Mts.Surface000).padding(horizontal = 10.dp, vertical = 8.dp)) {
                c.ask.detail.forEach { Text(it, style = HqType.Caption.copy(fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace, fontSize = 12.sp, lineHeight = 17.sp), color = Mts.Ink) }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PrimaryButton("Allow once", Modifier.weight(1f)) { actions.onAllow(c.sessionId, c.ask) }
                OutlineButton("Not now", Modifier.weight(1f)) { actions.onNotNow(c.sessionId, c.ask) }
            }
            Text(HelperWords.allowHint(c.ask), style = SmallText, color = Mts.InkMuted)
        } else if (waiting) {
            // What it asks is too long to read here, or cannot be read at all: the owner reads all of it on the helper's own page.
            PrimaryButton("Open helper", Modifier.fillMaxWidth()) { actions.onOpen(c.id) }
        }
    }
}

@Composable
internal fun OutlineButton(label: String, modifier: Modifier = Modifier, enabled: Boolean = true, color: Color = Mts.Ink, onClick: () -> Unit) {
    Box(
        modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 24.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = color, maxLines = 1) }
}

@Composable
internal fun StatePill(state: HelperState) {
    val (color, spin) = when (state) {
        HelperState.NeedsOk -> Mts.Amber to false
        HelperState.Working -> Mts.BrandBlue to true
        HelperState.Offline, HelperState.Lost -> Mts.Amber to false
        else -> Mts.InkMuted to false
    }
    Row(
        Modifier.heightIn(min = 28.dp).clip(CircleShape).border(1.dp, if (color == Mts.InkMuted) Mts.Line else color, CircleShape).padding(horizontal = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        if (spin) Spinner(14, color)
        Text(HelperWords.stateWords(state), style = HqType.NavLabel, color = color, maxLines = 1)
    }
}

@Composable
private fun FinishedRow(past: List<HelperCard>, open: Boolean, onToggle: () -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(shape).background(Mts.Surface100).clickable(role = Role.Button, onClickLabel = if (open) "Hide finished" else "Show finished", onClick = onToggle).padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text("Finished", style = HqType.BodyBold, color = Mts.Ink)
            Text(listOf(past.first().title, past.first().time).filter { it.isNotBlank() }.joinToString(" · "), style = HqType.BodySm, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Text("${past.size}", style = HqType.BodySm, color = Mts.InkMuted)
        Icon(if (open) HqIcons.ChevronUp else HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

@Composable
private fun PastRow(c: HelperCard, onClick: () -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).border(1.dp, Mts.Hairline, shape).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp, vertical = 10.dp)) {
        Text(c.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
        Text(listOf(HelperWords.stateWords(c.state), c.agent, c.time).filter { it.isNotBlank() }.joinToString(" · "), style = HqType.BodySm, color = Mts.InkMuted, maxLines = 1)
        if (c.generic && c.folder.isNotBlank()) Text(c.folder, style = SmallText, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun StartButton(modifier: Modifier, onClick: () -> Unit) {
    Row(
        modifier.heightIn(min = 56.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button, onClick = onClick).padding(start = 20.dp, end = 24.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) { Icon(HqIcons.Plus, null, Modifier.size(24.dp), tint = Mts.OnGreen); Text("Start a helper", style = HqType.BodySmBold, color = Mts.OnGreen) }
}

@Composable
internal fun Snack(line: HelperLine, onDismiss: () -> Unit, modifier: Modifier) {
    LaunchedEffect(line) { delay(LINE_MS); onDismiss() }
    Row(
        modifier.padding(horizontal = 8.dp, vertical = 8.dp).fillMaxWidth().heightIn(min = 48.dp).background(Mts.Ink, RoundedCornerShape(6.dp)).padding(horizontal = 16.dp).semantics { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
    ) { Text(line.text, style = HqType.BodySm, color = Mts.Surface100) }
}
