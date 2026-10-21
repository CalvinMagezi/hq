package example.hq.spike.ui.tasks

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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
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
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import java.time.LocalDate
import kotlinx.coroutines.delay

private const val LINE_MS = 4_000L

/** The Tasks place: Today, Upcoming and All, with a checkbox on each row and swipes to finish or move. */
@Composable
fun TasksScreen(ui: TasksUi, actions: TasksActions) {
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Tabs(ui.tab, actions.onTab)
            Box(Modifier.weight(1f).fillMaxWidth()) { Feed(ui, actions) }
        }
        NewTaskButton(Modifier.align(Alignment.BottomEnd).padding(16.dp)) { actions.onQuick(QuickState()) }
        ui.line?.let { Snack(it, actions, Modifier.align(Alignment.BottomCenter).padding(bottom = 4.dp)) }
    }
}

@Composable
private fun Tabs(tab: TaskTab, onTab: (TaskTab) -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).drawBehind { drawLine(Mts.Hairline, Offset(0f, size.height), Offset(size.width, size.height), 1.dp.toPx()) }) {
        TaskTab.entries.forEach { t ->
            val on = t == tab
            Box(
                Modifier.weight(1f).heightIn(min = 48.dp).clickable(role = Role.Tab, onClickLabel = t.name) { onTab(t) }
                    .drawBehind { if (on) drawRoundRect(Mts.BrandGreen, Offset(16.dp.toPx(), size.height - 3.dp.toPx()), Size(size.width - 32.dp.toPx(), 3.dp.toPx()), CornerRadius(3.dp.toPx())) },
                contentAlignment = Alignment.Center,
            ) { Text(t.name, style = HqType.BodySmBold, color = if (on) Mts.Ink else Mts.InkMuted) }
        }
    }
}

@Composable
private fun Feed(ui: TasksUi, actions: TasksActions) {
    val groups = ui.groups
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
        if (ui.offline) item { Text("HQ is not running. These are the last tasks it had.", style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.padding(16.dp)) }
        if (groups.isEmpty() && ui.loaded && !ui.offline) item { Text(emptyWords(ui.tab), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }
        groups.forEach { g ->
            item(key = "h-${g.key}") { GroupHeader(g, g.key in ui.expanded, actions.onToggleGroup) }
            if (!g.collapsible || g.key in ui.expanded) items(g.tasks.size, key = { "${g.key}-${g.tasks[it].id}" }) { TaskRow(g.tasks[it], ui.today, actions) }
        }
    }
}

private fun emptyWords(tab: TaskTab) = when (tab) { TaskTab.Today -> "Nothing for today."; TaskTab.Upcoming -> "Nothing coming up."; TaskTab.All -> "No open tasks." }

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
    val swipe = rememberSwipeToDismissBoxState(confirmValueChange = {
        when (it) {
            SwipeToDismissBoxValue.StartToEnd -> { actions.onDone(t); true }
            SwipeToDismissBoxValue.EndToStart -> { actions.onTomorrow(t); true }
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
                Text(t.title, style = HqType.BodyBold, color = Mts.Ink)
                val meta = TasksLogic.meta(t, today)
                if (meta.isNotEmpty()) Text(
                    buildAnnotatedString { meta.forEach { r -> withStyle(SpanStyle(color = if (r.amber) Mts.Amber else Mts.InkMuted, fontWeight = if (r.bold) FontWeight.Bold else FontWeight.Normal)) { append(r.text) } } },
                    style = HqType.BodySm,
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
