package example.hq.spike.ui.inbox

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.delay

private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private const val LINE_MS = 4_000L

private fun CardIcon.vector(): ImageVector = when (this) {
    CardIcon.Sparkle -> HqIcons.Spark
    CardIcon.Info -> HqIcons.Info
    CardIcon.Pause -> HqIcons.Pause
    CardIcon.Check -> HqIcons.Check
    CardIcon.Task -> HqIcons.Tasks
    CardIcon.Warning -> HqIcons.Warning
}

/** The Inbox place: what needs the owner, what HQ reports, and what was decided. */
@Composable
fun InboxScreen(ui: InboxUi, actions: InboxActions, onTakeLook: (InboxCard) -> Unit) {
    Column(Modifier.fillMaxSize()) {
        Tabs(ui, actions.onTab)
        Box(Modifier.weight(1f).fillMaxWidth()) {
            when (ui.tab) {
                InboxTab.Needs -> NeedsFeed(ui, actions, onTakeLook)
                InboxTab.Updates -> UpdatesFeed(ui, actions)
                InboxTab.Done -> DoneFeed(ui)
            }
            ui.line?.let { Snack(it, actions, Modifier.align(Alignment.BottomCenter)) }
        }
    }
}

@Composable
private fun Tabs(ui: InboxUi, onTab: (InboxTab) -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }) {
        listOf(
            Triple(InboxTab.Needs, "Needs you", ui.shownNeeds.size), Triple(InboxTab.Updates, "Updates", ui.shownUpdates.size), Triple(InboxTab.Done, "Done", 0),
        ).forEach { (tab, label, count) ->
            val on = ui.tab == tab
            Box(
                Modifier.weight(1f).heightIn(min = 48.dp).clickable(role = Role.Tab, onClickLabel = label) { onTab(tab) }
                    .drawBehind { if (on) drawRoundRect(Mts.BrandGreen, Offset(16.dp.toPx(), size.height - 3.dp.toPx()), Size(size.width - 32.dp.toPx(), 3.dp.toPx()), CornerRadius(3.dp.toPx())) },
                contentAlignment = Alignment.Center,
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(label, style = HqType.BodySmBold, color = if (on) Mts.Ink else Mts.InkMuted, maxLines = 1)
                    if (count > 0) {
                        Box(Modifier.heightIn(min = 20.dp).widthIn20().background(if (on) Mts.BrandGreen else Mts.Surface200, RoundedCornerShape(10.dp)).padding(horizontal = 6.dp), contentAlignment = Alignment.Center) {
                            Text("$count", style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold, lineHeight = 20.sp), color = if (on) Mts.OnGreen else Mts.Ink)
                        }
                    }
                }
            }
        }
    }
}

private fun Modifier.widthIn20() = this.then(Modifier.defaultMinSize(minWidth = 20.dp))

@Composable
private fun NeedsFeed(ui: InboxUi, actions: InboxActions, onTakeLook: (InboxCard) -> Unit) {
    val cards = ui.shownNeeds
    if (cards.isEmpty() && ui.loaded) {
        CaughtUp(ui)
        return
    }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, top = 12.dp, end = 16.dp, bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (ui.offline) item { Text("HQ is not running. These are the last things it asked.", style = HqType.BodySm, color = Mts.Amber) }
        items(cards, key = { it.id }) { card -> NeedCard(card, ui.answering[card.id], ui.offline, actions, onTakeLook) }
    }
}

@Composable
private fun NeedCard(card: InboxCard, answer: Answer?, offline: Boolean, actions: InboxActions, onTakeLook: (InboxCard) -> Unit) {
    val shape = RoundedCornerShape(16.dp)
    val waiting = card.kind == CardKind.HelperAsks
    Column(
        Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, if (waiting) Mts.Amber else Mts.Hairline, shape)
            .clickable(role = Role.Button, onClickLabel = "Details") { actions.onDetails(card) }.padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Avatar(card.icon.vector())
            Column(Modifier.weight(1f)) {
                Text(card.title, style = HqType.BodyBold.copy(fontSize = 17.sp), color = Mts.Ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(listOf(card.where, InboxWords.ago(card.createdAt)).filter { it.isNotBlank() }.joinToString(" · "), style = SmallText, color = Mts.InkMuted, maxLines = 1)
            }
        }
        Text(card.body, style = HqType.BodySm.copy(fontSize = 15.sp, lineHeight = 22.sp), color = Mts.Ink)
        (answer as? Answer.Failed)?.let { Text(it.reason, style = HqType.BodySm, color = Mts.Amber) }
        AnswerRow(card, answer, offline, actions, onTakeLook)
    }
}

@Composable
private fun AnswerRow(card: InboxCard, answer: Answer?, offline: Boolean, actions: InboxActions, onTakeLook: (InboxCard) -> Unit, modifier: Modifier = Modifier) {
    val sending = answer == Answer.Sending
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        if (card.canAllow) {
            PrimaryButton(if (sending) "Answering…" else card.allowLabel, Modifier.weight(1f), enabled = !sending && !offline) { actions.onAllow(card) }
            TextButton("Not now", enabled = !sending && !offline) { actions.onNotNow(card) }
        } else if (card.kind == CardKind.Approval) {
            // Too long to approve from a card: it is read in full on the details page first.
            PrimaryButton("Open", Modifier.weight(1f)) { actions.onDetails(card) }
        } else {
            PrimaryButton(card.allowLabel.ifBlank { "Take a look" }, Modifier.weight(1f)) { onTakeLook(card) }
        }
    }
}

@Composable
internal fun PrimaryButton(label: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    Box(
        modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).alpha(if (enabled) 1f else 0.5f).clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 24.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = Mts.OnGreen, maxLines = 1) }
}

@Composable
internal fun TextButton(label: String, enabled: Boolean = true, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).alpha(if (enabled) 1f else 0.5f).clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
        Text(label, style = HqType.BodySmBold, color = Mts.InkMuted, maxLines = 1)
    }
}

@Composable
private fun Avatar(icon: ImageVector) {
    Box(Modifier.size(40.dp).background(Mts.Surface200, CircleShape), contentAlignment = Alignment.Center) { Icon(icon, null, Modifier.size(22.dp), tint = Mts.Ink) }
}

/** Nothing waits on the owner: a check, when it was last looked, and what was decided today. */
@Composable
private fun CaughtUp(ui: InboxUi) {
    val today = ui.done.filter { InboxWords.sameDay(it.createdAt) }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
        item {
            Column(Modifier.fillMaxWidth().padding(start = 24.dp, top = 48.dp, end = 24.dp, bottom = 28.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Box(Modifier.size(64.dp).background(Mts.Surface200, CircleShape), contentAlignment = Alignment.Center) { Icon(HqIcons.Check, null, Modifier.size(32.dp), tint = Mts.BrandGreen) }
                Text("You're all caught up", style = HqType.H2, color = Mts.Ink)
                Text(if (ui.checkedAt.isNotBlank()) "Nothing is waiting on you. Checked at ${ui.checkedAt}." else "Nothing is waiting on you.", style = HqType.BodySm, color = Mts.InkMuted)
            }
        }
        if (today.isNotEmpty()) {
            item { Text("You decided today".uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(bottom = 6.dp)) }
            item { DoneList(today) }
        }
    }
}

@Composable
private fun DoneList(entries: List<DoneEntry>) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100)) {
        entries.forEachIndexed { i, e ->
            Column(Modifier.fillMaxWidth().heightIn(min = 64.dp).drawBehind { if (i > 0) drawLine(Mts.Hairline, Offset(0f, 0f), Offset(size.width, 0f), 1.dp.toPx()) }.padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.Center) {
                Text(e.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(InboxWords.clock(e.createdAt), style = HqType.BodySm, color = Mts.InkMuted)
            }
        }
    }
}

@Composable
private fun DoneFeed(ui: InboxUi) {
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp)) {
        if (ui.done.isEmpty()) item { Text("Nothing decided yet.", style = HqType.BodySm, color = Mts.InkMuted) } else item { DoneList(ui.done) }
    }
}

@Composable
private fun UpdatesFeed(ui: InboxUi, actions: InboxActions) {
    val rows = ui.shownUpdates
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
        if (rows.isNotEmpty()) item {
            Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), horizontalArrangement = Arrangement.End) {
                Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onClearAll).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) { Text("Clear all updates", style = HqType.BodySmBold, color = Mts.BrandBlue) }
            }
        }
        if (rows.isEmpty() && ui.loaded) item { Text("No updates.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }
        val groups = rows.groupBy { if (InboxWords.sameDay(it.createdAt)) "Today" else "Earlier" }
        groups.forEach { (label, group) ->
            item(key = "g-$label") { Text(label.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp, bottom = 8.dp)) }
            items(group, key = { it.id }) { UpdateRow(it, actions) }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun UpdateRow(card: InboxCard, actions: InboxActions) {
    val swipe = rememberSwipeToDismissBoxState(confirmValueChange = { (it == SwipeToDismissBoxValue.EndToStart).also { clear -> if (clear) actions.onClear(card) } })
    SwipeToDismissBox(
        swipe, enableDismissFromStartToEnd = false,
        backgroundContent = {
            if (swipe.dismissDirection == SwipeToDismissBoxValue.EndToStart) {
                Box(Modifier.fillMaxSize().background(Mts.BrandIndigo), contentAlignment = Alignment.CenterEnd) {
                    Column(Modifier.width(104.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Icon(HqIcons.Archive, null, Modifier.size(24.dp), tint = Mts.Ink)
                        Text("Clear", style = HqType.NavLabel, color = Mts.Ink)
                    }
                }
            }
        },
    ) {
        Row(
            Modifier.fillMaxWidth().background(Mts.Surface000).drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }
                .padding(horizontal = 16.dp, vertical = 14.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.Top,
        ) {
            Avatar(card.icon.vector())
            Column(Modifier.weight(1f)) {
                Text(card.title, style = HqType.BodyBold, color = Mts.Ink)
                if (card.body.isNotBlank()) Text(card.body, style = HqType.BodySm, color = Mts.InkMuted)
            }
            Text(InboxWords.clock(card.createdAt), style = HqType.Caption.copy(lineHeight = 20.sp), color = Mts.InkMuted)
        }
    }
}

@Composable
private fun Snack(line: InboxLine, actions: InboxActions, modifier: Modifier) {
    LaunchedEffect(line) { if (line.action == null) { delay(LINE_MS); actions.onDismissLine() } }
    Row(
        modifier.padding(horizontal = 8.dp, vertical = 8.dp).fillMaxWidth().heightIn(min = 48.dp).background(Mts.Ink, RoundedCornerShape(6.dp)).padding(start = 16.dp).semantics { liveRegion = LiveRegionMode.Polite },
        horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(line.text, style = HqType.BodySm, color = Mts.Surface100, modifier = Modifier.weight(1f))
        line.action?.let { Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onUndo).padding(horizontal = 16.dp), contentAlignment = Alignment.Center) { Text(it, style = HqType.BodySmBold, color = Mts.UndoBlue) } }
    }
}

/** A question on its own page: what is asked, what each answer does, who asked and when, and the answers at the foot. */
@Composable
fun InboxDetails(card: InboxCard, answer: Answer?, offline: Boolean, actions: InboxActions) {
    BackHandler { actions.onDetails(null) }
    Column(Modifier.fillMaxSize().background(Mts.Surface000).statusBarsPadding().navigationBarsPadding()) {
        Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconAction(HqIcons.ArrowBack, "Back", iconSize = 24.dp) { actions.onDetails(null) }
            Text("Details", style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.padding(start = 4.dp))
        }
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 8.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text(card.title, style = HqType.H2.copy(fontSize = 26.sp, lineHeight = 32.sp), color = Mts.Ink)
            Text(card.body, style = HqType.Body, color = Mts.InkMuted)
            if (card.canAllow || card.kind == CardKind.Approval) {
                Outcome(HqIcons.Check, Mts.BrandBlue, "If you allow it", "HQ records your yes. What asked then goes ahead.")
                Outcome(HqIcons.Pause, Mts.InkMuted, "If you say not now", "Nothing is done.")
            }
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                KeyValue("Asked by", card.asked)
                KeyValue("When", InboxWords.clock(card.createdAt).let { if (it.isBlank()) InboxWords.ago(card.createdAt) else if (InboxWords.sameDay(card.createdAt)) "$it today" else it })
            }
        }
        Column(Modifier.fillMaxWidth().background(Mts.Surface100).drawBehind { drawLine(Mts.Hairline, Offset.Zero, Offset(size.width, 0f), 1.dp.toPx()) }.padding(start = 16.dp, top = 12.dp, end = 16.dp, bottom = 24.dp)) {
            (answer as? Answer.Failed)?.let { Text(it.reason, style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.padding(bottom = 8.dp)) }
            AnswerRow(card, answer, offline, actions, onTakeLook = { actions.onDetails(null) })
        }
    }
}

@Composable
private fun Outcome(icon: ImageVector, tint: androidx.compose.ui.graphics.Color, head: String, body: String) {
    val shape = RoundedCornerShape(12.dp)
    Row(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Icon(icon, null, Modifier.size(22.dp), tint = tint)
        Column { Text(head, style = HqType.BodySmBold, color = Mts.Ink); Text(body, style = HqType.BodySm, color = Mts.InkMuted) }
    }
}

@Composable
private fun KeyValue(key: String, value: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(key, style = HqType.BodySmBold, color = Mts.Ink, modifier = Modifier.width(90.dp))
        Text(value, style = HqType.BodySm, color = Mts.InkMuted)
    }
}
