package example.hq.spike.ui.tasks

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.shell.consumeTouches
import example.hq.spike.ui.shell.rememberWords
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import java.time.Instant

/** The card at the top of Today: what HQ's checks did in the last 24 hours. A server HQ adds its own part under its own heading. */
@Composable
fun AwayCard(ui: AwayUi, tasks: List<TaskItem>, onOpen: () -> Unit) {
    if (!ui.hasNews) return
    val w = rememberWords()
    val assignee = { id: String -> tasks.firstOrNull { it.id == id }?.assignees?.firstOrNull() }
    val phone = ui.phone?.takeIf { it.hasNews }?.let { AwayWords.card(w, it, assignee) }.orEmpty()
    val server = ui.server?.takeIf { it.hasNews }?.let { AwayWords.card(w, it, { null }) }.orEmpty()
    val open = stringResource(R.string.away_open_label)
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).clip(RoundedCornerShape(16.dp)).background(Mts.Surface100)
            .clickable(role = Role.Button, onClick = onOpen).semantics { contentDescription = open }.padding(16.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(stringResource(R.string.away_card_title), style = HqType.BodyBold, color = Mts.Ink)
            Text(stringResource(R.string.away_card_hours), style = HqType.Caption, color = Mts.InkMuted)
            phone.forEach { Text(it, style = HqType.BodySm, color = Mts.Ink) }
            if (server.isNotEmpty()) {
                Text(stringResource(R.string.away_server_heading), style = HqType.BodySmBold, color = Mts.InkMuted, modifier = Modifier.padding(top = 4.dp))
                server.forEach { Text(it, style = HqType.BodySm, color = Mts.Ink) }
            }
        }
        Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

/** While you were away, the page: the briefing's counts by what happened, with a way to each task and initiative. */
@Composable
fun AwayPage(ui: AwayUi, tasks: List<TaskItem>, actions: TasksActions, onTask: (String) -> Unit) {
    BackHandler { actions.onAway(false) }
    val w = rememberWords()
    val assignee = { id: String -> tasks.firstOrNull { it.id == id }?.assignees?.firstOrNull() }
    Column(Modifier.fillMaxSize().background(Mts.Surface000).consumeTouches().statusBarsPadding()) {
        Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconAction(HqIcons.ArrowBack, stringResource(R.string.search_back), iconSize = 24.dp) { actions.onAway(false) }
            Column(Modifier.weight(1f).padding(start = 4.dp)) {
                Text(stringResource(R.string.away_card_title), style = HqType.BodyBold, color = Mts.Ink)
                ui.phoneAtMs?.let { Text(stringResource(R.string.away_page_line, w.clock(Instant.ofEpochMilli(it))), style = HqType.Caption, color = Mts.InkMuted) }
            }
        }
        LazyColumn(Modifier.weight(1f), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 24.dp)) {
            if (ui.phone == null) ui.phoneFailure?.let { item { Text(stringResource(it), style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) } }
            val showServerHeading = ui.server != null || ui.serverStale
            if (showServerHeading) item { Heading(stringResource(R.string.away_phone_heading)) }
            ui.phone?.let { part(it, assignee, actions, onTask, w) }
            if (showServerHeading) {
                item { Heading(stringResource(R.string.away_server_heading)) }
                if (ui.serverStale) {
                    item { Text(stringResource(R.string.away_stale), style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) }
                    ui.serverAtMs?.let { at -> item { Text(stringResource(R.string.away_saved_at, w.clock(Instant.ofEpochMilli(at))), style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 16.dp)) } }
                }
                ui.server?.let { part(it, { null }, actions, onTask, w, ui.serverHost) }
            }
            val waiting = ui.phone?.waitingOnYou ?: 0
            if (waiting > 0) {
                item { Section(R.string.away_sec_waiting, waiting) }
                item {
                    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(pluralStringResource(R.plurals.away_waiting_line, waiting, waiting), style = HqType.BodySm, color = Mts.Ink)
                        Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button, onClick = actions.onAwayToday).padding(horizontal = 24.dp), contentAlignment = Alignment.Center) {
                            Text(stringResource(R.string.away_go_today), style = HqType.BodySmBold, color = Mts.OnGreen)
                        }
                    }
                }
            }
            item { Text(stringResource(R.string.away_footer), style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(16.dp)) }
        }
    }
}

private fun androidx.compose.foundation.lazy.LazyListScope.part(a: Away, assignee: (String) -> String?, actions: TasksActions, onTask: (String) -> Unit, w: example.hq.spike.ui.shell.Words, host: String? = null) {
    val now = Instant.now()
    if (a.windows.isNotEmpty()) {
        item { Section(R.string.away_sec_soon, a.windows.size) }
        a.windows.forEach { win -> item(key = "w-${host}-${win.taskId}") { Row2("${win.displayId} · ${win.title}", AwayWords.finishes(w, win.releaseAtSec, now)) { onTask(win.taskId) } } }
    }
    if (a.finishedTotal > 0) {
        item { Section(R.string.away_sec_done, a.finishedTotal) }
        a.finished.forEach { f -> item(key = "f-${host}-${f.initiativeId}") { Row2(w.n(R.plurals.away_initiative_row, f.count, f.name, f.count), host) { actions.onAwayInitiative(f.initiativeId) } } }
    }
    if (a.reopenedTotal > 0) {
        item { Section(R.string.away_sec_back, a.reopenedTotal) }
        a.reopened.forEach { r ->
            val helper = AwayWords.sendBackTo(assignee(r.taskId))
            item(key = "r-${host}-${r.taskId}") { Row2("${r.displayId} · ${r.title}", if (helper == null) w.s(R.string.away_back_row_you) else w.s(R.string.away_back_row_helper, helper)) { onTask(r.taskId) } }
        }
    }
}

@Composable
private fun Heading(text: String) = Text(text, style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 4.dp))

@Composable
private fun Section(label: Int, count: Int) {
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(stringResource(label), style = HqType.Group, color = Mts.InkMuted)
        Box(Modifier.clip(CircleShape).background(Mts.Surface100).padding(horizontal = 8.dp, vertical = 2.dp)) { Text("$count", style = HqType.Badge, color = Mts.InkMuted) }
    }
}

@Composable
private fun Row2(title: String, line: String?, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp, vertical = 10.dp)) {
        Text(title, style = HqType.BodyBold, color = Mts.Ink)
        if (!line.isNullOrBlank()) Text(line, style = HqType.BodySm, color = Mts.InkMuted)
    }
}
