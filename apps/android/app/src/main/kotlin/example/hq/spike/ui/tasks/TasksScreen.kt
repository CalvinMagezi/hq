package example.hq.spike.ui.tasks

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.ui.chat.ActionRow
import example.hq.spike.ui.chat.SheetFrame
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.webhost.LocalOpenWeb
import example.hq.spike.ui.webhost.WebPage
import example.hq.spike.ui.theme.Mts
import java.time.LocalDate
import kotlinx.coroutines.delay

private const val LINE_MS = 4_000L
private const val TITLE_LINES = 2
private const val META_LINES = 2

/** The Tasks place: Today, Upcoming and All, with a checkbox on each row and swipes to finish or move. */
@Composable
fun TasksScreen(ui: TasksUi, actions: TasksActions, onHelpers: () -> Unit = {}) {
    androidx.activity.compose.BackHandler(enabled = ui.selected.isNotEmpty(), onBack = actions.onClearSelection)
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            if (ui.selected.isEmpty()) Tabs(ui.tab, actions.onTab) else SelectionBar(ui, actions)
            Box(Modifier.weight(1f).fillMaxWidth()) { Feed(ui, actions, onHelpers) }
        }
        if (ui.selected.isEmpty()) NewTaskButton(Modifier.align(Alignment.BottomEnd).padding(16.dp)) { actions.onQuick(QuickState()) }
        ui.bulkAsk?.let { ask ->
            val words = BulkEdit.confirmDone(ui.selected.size)
            SheetFrame({ actions.onBulkAsk(null) }, top = null) {
                Text(words.title, style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 4.dp))
                Text(words.body, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp))
                ActionRow(HqIcons.Check, words.confirm, null, { actions.onBulkAsk(ask) })
                ActionRow(HqIcons.Close, "Cancel", null, { actions.onBulkAsk(null) })
            }
        }
        when (ui.bulkPrompt) {
            "status" -> SheetFrame({ actions.onBulkPrompt(null) }, top = null) {
                BulkEdit.STATUS_CHOICES.forEach { s -> ActionRow(HqIcons.Check, TasksLogic.statusWords(s), null, { actions.onBulk(Bulk.Status(s)) }) }
            }
            "priority" -> SheetFrame({ actions.onBulkPrompt(null) }, top = null) {
                BulkEdit.PRIORITY_CHOICES.forEach { (key, label) -> ActionRow(HqIcons.Check, label, null, { actions.onBulk(Bulk.Priority(key)) }) }
            }
        }
        ui.line?.let { Snack(it, actions, Modifier.align(Alignment.BottomCenter).padding(bottom = 4.dp)) }
    }
}

@Composable
private fun Tabs(tab: TaskTab, onTab: (TaskTab) -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }) {
        TaskTab.entries.forEach { t ->
            val on = t == tab
            Box(
                Modifier.weight(1f).heightIn(min = 48.dp).selectable(selected = on, role = Role.Tab, onClick = { onTab(t) })
                    .drawBehind { if (on) drawRoundRect(Mts.BrandGreen, Offset(16.dp.toPx(), size.height - 3.dp.toPx()), Size(size.width - 32.dp.toPx(), 3.dp.toPx()), CornerRadius(3.dp.toPx())) },
                contentAlignment = Alignment.Center,
            ) { Text(t.name, style = HqType.BodySmBold, color = if (on) Mts.Ink else Mts.InkMuted) }
        }
    }
}

@Composable
private fun Feed(ui: TasksUi, actions: TasksActions, onHelpers: () -> Unit) {
    val groups = ui.groups
    val onAll = ui.tab == TaskTab.All
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
        if (ui.offline) item { Text("HQ is not running. These are the last tasks it had.", style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.padding(16.dp)) }
        if (onAll) item(key = "chips") { AllChips(ui, actions.onChip) }
        if (onAll && ui.sessions.isNotEmpty() && ui.chip == AllChip.Active) item(key = "strip") { WorkingStrip(ui.sessions, onHelpers) }
        if (groups.isEmpty() && ui.loaded && !ui.offline) item { Text(if (onAll && ui.chip == AllChip.Archived && !ui.archivedLoaded) "Asking HQ\u2026" else if (onAll) TasksLogic.emptyAll(ui.chip) else emptyWords(ui.tab), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }
        groups.forEach { g ->
            if (g.title.isNotBlank()) item(key = "h-${g.key}") { GroupHeader(g, g.key in ui.expanded, actions.onToggleGroup) }
            val open = g.key in ui.expanded
            val shown = if (g.preview != null && !open) g.tasks.take(g.preview) else g.tasks
            if (!g.collapsible || open) items(shown.size, key = { "${g.key}-${shown[it].id}" }) {
                if (onAll) AllRow(shown[it], ui.today, shown[it].id in ui.workingIds, ui.selected, ui.chip != AllChip.Archived, actions) else TaskRow(shown[it], ui.today, actions)
            }
            if (g.preview != null && g.tasks.size > g.preview) item(key = "m-${g.key}") { MoreRow(g.tasks.size - g.preview, open) { actions.onToggleGroup(g.key) } }
        }
        if (onAll && ui.loaded) item(key = "web") { WebViews() }
    }
}

/** The end of the All tab: the web app's Board and Timeline, shown in the app's own screen. */
@Composable
private fun WebViews() {
    val open = LocalOpenWeb.current
    Column(Modifier.fillMaxWidth().padding(top = 16.dp)) {
        listOf(WebPage.Board to "Board", WebPage.Timeline to "Timeline").forEach { (page, label) ->
            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button) { open(page) }.padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(label, style = HqType.BodySmBold, color = Mts.BrandBlue, modifier = Modifier.weight(1f))
                Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
        }
    }
}

private fun emptyWords(tab: TaskTab) = when (tab) { TaskTab.Today -> "Nothing for today."; TaskTab.Upcoming -> "Nothing coming up."; TaskTab.All -> "No open tasks." }

/** The All tab's lists as chips; Stuck and Ready to check carry how many wait. */
@Composable
private fun AllChips(ui: TasksUi, onChip: (AllChip) -> Unit) {
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        AllChip.entries.forEach { c ->
            val on = c == ui.chip
            val n = TasksLogic.chipCount(ui.scoped, c)
            Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).selectable(selected = on, role = Role.Tab, onClick = { onChip(c) }), contentAlignment = Alignment.Center) {
                Row(
                    Modifier.heightIn(min = 36.dp).clip(CircleShape).then(if (on) Modifier.background(Mts.BrandGreen, CircleShape).border(1.dp, Mts.BrandGreen, CircleShape) else Modifier.border(1.dp, Mts.Line, CircleShape)).padding(horizontal = 14.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (on) Icon(HqIcons.Check, null, Modifier.size(16.dp), tint = Mts.OnGreen)
                    Text(if (n > 0) "${c.label} $n" else c.label, style = HqType.BodySmBold, color = if (on) Mts.OnGreen else Mts.Ink, maxLines = 1)
                }
            }
        }
    }
}

/** "1 helper working": which helper and where, and a way to the Helpers place. */
@Composable
private fun WorkingStrip(sessions: List<example.hq.spike.data.HqClient.WorkSession>, onOpen: () -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    val first = sessions.first()
    val count = sessions.size
    Row(
        Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp).fillMaxWidth().heightIn(min = 56.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).clickable(role = Role.Button, onClick = onOpen).padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(8.dp).background(Mts.Amber, CircleShape))
        Text(
            buildAnnotatedString {
                withStyle(SpanStyle(color = Mts.Ink, fontWeight = FontWeight.Bold)) { append(if (count == 1) "1 helper working" else "$count helpers working") }
                val where = first.cwd.trimEnd('/').substringAfterLast('/').ifBlank { first.harness }
                withStyle(SpanStyle(color = Mts.InkMuted)) { append(" \u00b7 ${first.actor.ifBlank { first.harness }} on $where") }
            },
            style = HqType.BodySm, modifier = Modifier.weight(1f), maxLines = 2, overflow = TextOverflow.Ellipsis,
        )
        Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

/** A task on the All tab: its number and what stands out, its title, and what it is waiting on. No checkbox and no swipe: it is opened to be changed. */
@Composable
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
private fun AllRow(t: TaskItem, today: LocalDate, working: Boolean, selected: Set<String>, bulkable: Boolean, actions: TasksActions) {
    val tag = TasksLogic.idTag(t)
    val meta = TasksLogic.allMeta(t, today, working)
    val selecting = selected.isNotEmpty()
    val on = t.id in selected
    Row(
        Modifier.fillMaxWidth().heightIn(min = 72.dp).then(if (on) Modifier.background(Mts.Surface100) else Modifier)
            .semantics { this.selected = on }
            .combinedClickable(role = Role.Button, onLongClickLabel = if (bulkable) "Select" else null, onLongClick = if (bulkable) ({ actions.onSelect(t.id) }) else null) { if (selecting) actions.onSelect(t.id) else actions.onOpen(t) }
            .drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }.padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (selecting) Box(
            Modifier.padding(top = 2.dp).size(22.dp).clip(RoundedCornerShape(6.dp)).then(if (on) Modifier.background(Mts.BrandGreen) else Modifier.border(2.dp, Mts.Line, RoundedCornerShape(6.dp))),
            contentAlignment = Alignment.Center,
        ) { if (on) Icon(HqIcons.Check, null, Modifier.size(16.dp), tint = Mts.OnGreen) }
        Column(Modifier.weight(1f)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(t.displayId, style = HqType.Caption.copy(fontWeight = FontWeight.Bold), color = Mts.InkMuted)
                tag?.let { Text(it.text, style = HqType.Caption.copy(fontWeight = FontWeight.Bold), color = if (it.danger) Mts.DangerText else Mts.Amber) }
            }
            Text(t.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = TITLE_LINES, overflow = TextOverflow.Ellipsis)
            if (meta.isNotEmpty()) Text(meta, style = HqType.BodySm, color = Mts.InkMuted, maxLines = META_LINES, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 2.dp))
        }
    }
}

/** Replaces the tabs while tasks are picked: how many, a way to pick them all or none, and what to do to them. */
@Composable
private fun SelectionBar(ui: TasksUi, actions: TasksActions) {
    val ids = ui.groups.flatMap { g -> g.tasks.map { it.id } }
    val all = ui.selected.size >= ids.size
    Column(Modifier.fillMaxWidth().drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }.padding(bottom = 8.dp)) {
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
            IconAction(HqIcons.Close, "Clear selection", iconSize = 24.dp, onClick = actions.onClearSelection)
            Text("${ui.selected.size} selected", style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f).padding(start = 4.dp))
            Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button) { if (all) actions.onClearSelection() else actions.onSelectAll(ids) }.padding(horizontal = 16.dp), contentAlignment = Alignment.Center) {
                Text(if (all) "Clear" else "Select all ${ids.size}", style = HqType.BodySmBold, color = Mts.BrandBlue)
            }
        }
        Row(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("Set status" to "status", "Set priority" to "priority").forEach { (label, prompt) ->
                Box(
                    Modifier.heightIn(min = 40.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(enabled = !ui.bulking, role = Role.Button) { actions.onBulkPrompt(prompt) }.padding(horizontal = 16.dp),
                    contentAlignment = Alignment.Center,
                ) { Text(if (ui.bulking) "Working\u2026" else label, style = HqType.BodySmBold, color = Mts.Ink) }
            }
        }
    }
}

/** "Show 69 more" under a long group, and "Show fewer" once it is open. */
@Composable
private fun MoreRow(hidden: Int, open: Boolean, onToggle: () -> Unit) {
    Box(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onToggle).padding(horizontal = 16.dp), contentAlignment = Alignment.CenterStart) {
        Text(if (open) "Show fewer" else "Show $hidden more", style = HqType.BodySmBold, color = Mts.BrandBlue)
    }
}

@Composable
private fun GroupHeader(g: TaskGroup, open: Boolean, onToggle: (String) -> Unit) {
    val mod = if (g.collapsible) Modifier.clickable(role = Role.Button, onClickLabel = if (open) "Hide" else "Show") { onToggle(g.key) } else Modifier
    Row(mod.fillMaxWidth().padding(start = 16.dp, top = if (g.badge) 16.dp else 18.dp, end = 16.dp, bottom = if (g.badge) 8.dp else 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(g.title.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted)
        if (g.badge) Box(Modifier.defaultMinSize(minWidth = 20.dp, minHeight = 20.dp).background(Mts.Surface200, RoundedCornerShape(10.dp)).padding(horizontal = 6.dp), contentAlignment = Alignment.Center) {
            Text("${g.tasks.size}", style = HqType.Caption.copy(fontWeight = FontWeight.Bold, lineHeight = 20.sp), color = Mts.Ink)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TaskRow(t: TaskItem, today: LocalDate, actions: TasksActions) {
    val latest by rememberUpdatedState(t)
    val swipe = rememberSwipeToDismissBoxState(confirmValueChange = {
        when (it) {
            SwipeToDismissBoxValue.StartToEnd -> { actions.onDone(latest); true }
            SwipeToDismissBoxValue.EndToStart -> { actions.onTomorrow(latest); true }
            else -> false
        }
    })
    SwipeToDismissBox(
        swipe,
        backgroundContent = {
            val right = swipe.dismissDirection == SwipeToDismissBoxValue.StartToEnd
            val left = swipe.dismissDirection == SwipeToDismissBoxValue.EndToStart
            if (right || left) Box(Modifier.fillMaxSize().background(if (right) Mts.BrandGreen else Mts.BrandIndigo), contentAlignment = if (right) Alignment.CenterStart else Alignment.CenterEnd) {
                Column(Modifier.width(96.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Icon(if (right) HqIcons.Check else HqIcons.Redo, null, Modifier.size(24.dp), tint = if (right) Mts.OnGreen else Mts.Ink)
                    Text(if (right) "Done" else "Tomorrow", style = HqType.NavLabel, color = if (right) Mts.OnGreen else Mts.Ink)
                }
            }
        },
    ) {
        Row(
            Modifier.fillMaxWidth().background(Mts.Surface000).drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }.clickable(role = Role.Button) { actions.onOpen(t) }
                .padding(start = 4.dp, end = 16.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Box(Modifier.size(48.dp).clickable(role = Role.Checkbox, onClickLabel = "Mark done") { actions.onDone(t) }.semantics { contentDescription = "Mark ${t.title} done" }, contentAlignment = Alignment.Center) {
                Box(Modifier.size(22.dp).border(2.dp, Mts.Line, RoundedCornerShape(6.dp)))
            }
            Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                Text(t.title, style = HqType.BodyBold, color = Mts.Ink, maxLines = TITLE_LINES, overflow = TextOverflow.Ellipsis)
                val meta = TasksLogic.meta(t, today)
                if (meta.isNotEmpty()) Text(
                    buildAnnotatedString { meta.forEach { r -> withStyle(SpanStyle(color = if (r.amber) Mts.Amber else Mts.InkMuted, fontWeight = if (r.bold) FontWeight.Bold else FontWeight.Normal)) { append(r.text) } } },
                    style = HqType.BodySm, maxLines = META_LINES, overflow = TextOverflow.Ellipsis,
                )
            }
            if (t.status == "ready_for_review") Box(
                Modifier.padding(top = 14.dp, start = 8.dp).heightIn(min = 36.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button) { actions.onOpen(t) }.padding(horizontal = 14.dp),
                contentAlignment = Alignment.Center,
            ) { Text("Check it", style = HqType.BodySmBold, color = Mts.Ink) }
        }
    }
}

@Composable
private fun NewTaskButton(modifier: Modifier, onClick: () -> Unit) {
    Row(
        modifier.heightIn(min = 56.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button, onClick = onClick).padding(start = 20.dp, end = 24.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) { Icon(HqIcons.Plus, null, Modifier.size(24.dp), tint = Mts.OnGreen); Text("New task", style = HqType.BodySmBold, color = Mts.OnGreen) }
}

@Composable
internal fun Snack(line: TaskLine, actions: TasksActions, modifier: Modifier) {
    LaunchedEffect(line) { delay(LINE_MS); actions.onDismissLine() }
    Row(
        modifier.padding(horizontal = 8.dp, vertical = 8.dp).fillMaxWidth().heightIn(min = 48.dp).background(Mts.Ink, RoundedCornerShape(6.dp)).padding(start = 16.dp).semantics { liveRegion = LiveRegionMode.Polite },
        horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(line.text, style = HqType.BodySm, color = Mts.Surface100, modifier = Modifier.weight(1f))
        line.action?.let { Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onUndo).padding(horizontal = 16.dp), contentAlignment = Alignment.Center) { Text(it, style = HqType.BodySmBold, color = Mts.UndoBlue) } }
    }
}
