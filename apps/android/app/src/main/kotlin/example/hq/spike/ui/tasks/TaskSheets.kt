package example.hq.spike.ui.tasks

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.input.OffsetMapping
import androidx.compose.ui.text.input.TransformedText
import androidx.compose.ui.text.input.VisualTransformation
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
import example.hq.spike.ui.theme.Mts
import java.time.LocalDate

private val Highlight = Color(0x4000ADEE)
private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private val PRIORITIES = listOf<Pair<String?, String>>(null to "None", "low" to "Low", "normal" to "Normal", "high" to "High", "urgent" to "Urgent")

/** Paints the words HQ understood on a wash, without changing the text. */
private class Understood(private val today: LocalDate) : VisualTransformation {
    override fun filter(text: AnnotatedString): TransformedText {
        val spans = QuickAdd.parse(text.text, today).understood
        val styled = buildAnnotatedString {
            append(text.text)
            spans.forEach { addStyle(SpanStyle(background = Highlight), it.range.first, it.range.last + 1) }
        }
        return TransformedText(styled, OffsetMapping.Identity)
    }
}

/** A new task in one line: the words HQ understood become chips, and Add task saves at once. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun QuickAddSheet(q: QuickState, today: LocalDate, actions: TasksActions) {
    val parsed = QuickAdd.parse(q.text, today)
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    val close = { actions.onQuick(null) }
    SheetFrame(close, top = if (q.more) 150.dp else 200.dp) {
        Column(Modifier.verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("New task", style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                IconAction(HqIcons.Close, "Close", iconSize = 24.dp, onClick = close)
            }
            val shape = RoundedCornerShape(12.dp)
            BasicTextField(
                value = q.text, onValueChange = { actions.onQuick(q.copy(text = it)) }, textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
                visualTransformation = remember(today) { Understood(today) },
                modifier = Modifier.fillMaxWidth().heightIn(min = 80.dp).clip(shape).background(Mts.Surface000).border(2.dp, Mts.BrandBlue, shape).padding(horizontal = 16.dp, vertical = 12.dp).focusRequester(focus),
                decorationBox = { inner -> Box { if (q.text.isEmpty()) Text("What needs doing?", style = HqType.Body, color = Mts.InkMuted); inner() } },
            )
            if (parsed.understood.isNotEmpty()) {
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    parsed.understood.forEach { u ->
                        Row(
                            Modifier.heightIn(min = 36.dp).clip(CircleShape).background(Mts.Surface200).clickable(role = Role.Button, onClickLabel = "Remove ${u.label}") { actions.onQuick(q.copy(text = without(q.text, u.range))) }.padding(start = 14.dp, end = 8.dp),
                            horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
                        ) { Text(u.label, style = HqType.BodySmBold, color = Mts.Ink); Icon(HqIcons.Close, null, Modifier.size(18.dp), tint = Mts.Ink) }
                    }
                }
                Text("Words HQ understood are chips. Tap one to remove it.", style = SmallText, color = Mts.InkMuted)
            }
            if (q.more) MoreFields(q, today, actions) else QuickPills(q, actions)
            q.error?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
            Box(
                Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).alpha(if (parsed.title.isBlank() || q.saving) 0.5f else 1f)
                    .clickable(enabled = parsed.title.isNotBlank() && !q.saving, role = Role.Button, onClick = actions.onQuickSave),
                contentAlignment = Alignment.Center,
            ) { Text(if (q.saving) "Adding…" else if (q.more) "Save" else "Add task", style = HqType.BodySmBold, color = Mts.OnGreen) }
        }
    }
}

/** Takes one understood word out of the text and tidies the gap. */
internal fun without(text: String, range: IntRange): String =
    (text.substring(0, range.first) + text.substring(range.last + 1)).replace(Regex("\\s{2,}"), " ").trim()

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun QuickPills(q: QuickState, actions: TasksActions) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Pill("Today", false) { actions.onQuick(q.copy(text = (q.text.trim() + " today").trim())) }
        Pill("Next week", false) { actions.onQuick(q.copy(text = (q.text.trim() + " next week").trim())) }
        Pill("Add details", false) { actions.onQuick(q.copy(more = true)) }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun MoreFields(q: QuickState, today: LocalDate, actions: TasksActions) {
    val shape = RoundedCornerShape(12.dp)
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Column(Modifier.fillMaxWidth().clip(shape).border(1.dp, Mts.Line, shape).background(Mts.Surface100).padding(horizontal = 16.dp, vertical = 8.dp)) {
            Text("Notes (optional)", style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
            BasicTextField(q.notes, { actions.onQuick(q.copy(notes = it)) }, Modifier.fillMaxWidth().heightIn(min = 48.dp), textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink))
        }
        Text("Priority", style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            PRIORITIES.forEach { (key, label) -> Pill(label, q.priority == key) { actions.onQuick(q.copy(priority = key)) } }
        }
        Text("Due", style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Pill("None", q.due == null) { actions.onQuick(q.copy(due = null)) }
            Pill("Today", q.due == today) { actions.onQuick(q.copy(due = today)) }
            Pill("Tomorrow", q.due == today.plusDays(1)) { actions.onQuick(q.copy(due = today.plusDays(1))) }
            Pill("Next week", q.due == TasksLogic.nextMonday(today)) { actions.onQuick(q.copy(due = TasksLogic.nextMonday(today))) }
        }
        Text("It goes to your Inbox list.", style = SmallText, color = Mts.InkMuted)
    }
}

@Composable
private fun Pill(label: String, selected: Boolean, onClick: () -> Unit) {
    Box(
        Modifier.heightIn(min = 36.dp).clip(CircleShape).then(if (selected) Modifier.background(Mts.Surface200).border(1.dp, Mts.BrandGreen, CircleShape) else Modifier.border(1.dp, Mts.Line, CircleShape))
            .clickable(role = Role.Button, onClick = onClick).padding(horizontal = 14.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1) }
}

/** A task on its own page. A task that hq finished leads with a card to mark it done or send it back. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TaskPage(d: TaskDetail, today: LocalDate, actions: TasksActions) {
    BackHandler { actions.onClose() }
    val t = d.task
    val menu = remember { androidx.compose.runtime.mutableStateOf(false) }
    val more = remember { androidx.compose.runtime.mutableStateOf(false) }
    Column(Modifier.fillMaxSize().background(Mts.Surface000).statusBarsPadding().navigationBarsPadding()) {
        Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 4.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconAction(HqIcons.ArrowBack, "Back", iconSize = 24.dp, onClick = actions.onClose)
            Text("Task", style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f).padding(start = 4.dp))
            IconAction(HqIcons.MoreVertical, "More", iconSize = 24.dp) { menu.value = true }
        }
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 8.dp, end = 16.dp, bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text(t.title, style = HqType.H2, color = Mts.Ink)
            if (t.status == "ready_for_review") ReadyCard(t, actions) else StatusRow(t, actions)
            val ready = t.status == "ready_for_review"
            if (t.description.isNotBlank()) {
                if (ready) Label("What was done")
                Text(t.description, style = HqType.Body, color = Mts.Ink)
            }
            d.error?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
            if (ready) MoreAbout(open = more.value, onToggle = { more.value = !more.value }) { Details(d, today, actions) } else Details(d, today, actions)
        }
    }
    if (menu.value) SheetFrame({ menu.value = false }, top = null) { ActionRow(HqIcons.Archive, "Archive", "It can be brought back with Undo.", { menu.value = false; actions.onArchive() }) }
    when (d.prompt) {
        "status" -> SheetFrame({ actions.onPrompt(null) }, top = null) {
            listOf("to_do", "in_progress", "blocked", "ready_for_review", "complete").forEach { s -> ActionRow(HqIcons.Check, TasksLogic.statusWords(s), null, { actions.onStatus(s) }, tint = if (s == t.status) Mts.BrandGreen else Mts.Ink) }
        }
        "priority" -> SheetFrame({ actions.onPrompt(null) }, top = null) {
            PRIORITIES.forEach { (key, label) -> ActionRow(HqIcons.Check, label, null, { actions.onPriority(key) }, tint = if (key == t.priority) Mts.BrandGreen else Mts.Ink) }
        }
    }
    if (d.prompt == "sendback") SendBackDialog(d, actions)
}

@Composable
private fun Details(d: TaskDetail, today: LocalDate, actions: TasksActions) {
    Facts(d.task, today)
    SubTasks(d, actions)
    Comments(d, actions)
}

/** A task waiting for a check keeps its detail folded away, so the card and what was done lead. */
@Composable
private fun MoreAbout(open: Boolean, onToggle: () -> Unit, content: @Composable () -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(shape).background(Mts.Surface100).clickable(role = Role.Button, onClickLabel = if (open) "Hide details" else "Show details", onClick = onToggle).padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text("More about this task", style = HqType.BodyBold, color = Mts.Ink)
            Text("Priority, dates, sub-tasks, comments, history", style = HqType.BodySm, color = Mts.InkMuted)
        }
        Icon(if (open) HqIcons.ChevronUp else HqIcons.ChevronDown, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
    if (open) content()
}

@Composable
private fun Label(text: String) = Text(text.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted)

@Composable
private fun ReadyCard(t: TaskItem, actions: TasksActions) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Amber, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text("Ready for you to check", style = HqType.BodyBold, color = Mts.Amber)
        Text("hq says this is finished${TasksLogic.readyAgo(t.readyAt, java.time.LocalDateTime.now(java.time.ZoneOffset.UTC))?.let { ", $it" } ?: ""}. Look it over, then mark it done.", style = HqType.BodySm, color = Mts.Ink)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(Modifier.weight(1f).heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button, onClick = actions.onMarkDone), contentAlignment = Alignment.Center) { Text("Mark done", style = HqType.BodySmBold, color = Mts.OnGreen) }
            Box(Modifier.weight(1f).heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button) { actions.onPrompt("sendback") }, contentAlignment = Alignment.Center) { Text("Send back", style = HqType.BodySmBold, color = Mts.Ink) }
        }
    }
}

@Composable
private fun StatusRow(t: TaskItem, actions: TasksActions) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(
            Modifier.weight(1f).heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).clickable(role = Role.Button, onClickLabel = "Change status") { actions.onPrompt("status") }.padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.size(10.dp).background(Mts.BrandBlue, CircleShape))
            Text(TasksLogic.statusWords(t.status), style = HqType.BodySmBold, color = Mts.Ink, modifier = Modifier.weight(1f))
            Icon(HqIcons.ChevronDown, null, Modifier.size(20.dp), tint = Mts.Ink)
        }
        Row(
            Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClickLabel = "Change priority") { actions.onPrompt("priority") }.padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(TasksLogic.priorityWords(t.priority) ?: "Priority", style = HqType.BodySmBold, color = if (t.priority == "high" || t.priority == "urgent") Mts.Amber else Mts.Ink)
            Icon(HqIcons.ChevronDown, null, Modifier.size(20.dp), tint = Mts.Ink)
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Facts(t: TaskItem, today: LocalDate) {
    val facts = listOfNotNull(t.assignees.firstOrNull()?.let { "for $it" }, t.tags.firstOrNull(), TasksLogic.parse(t.startDate)?.let { "Starts " + it.format(java.time.format.DateTimeFormatter.ofPattern("MMM d")) }, TasksLogic.parse(t.dueDate)?.let { "Due " + it.format(java.time.format.DateTimeFormatter.ofPattern("MMM d")) })
    if (facts.isEmpty()) return
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        facts.forEach { Box(Modifier.heightIn(min = 32.dp).clip(CircleShape).border(1.dp, Mts.Hairline, CircleShape).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) { Text(it, style = HqType.Caption.copy(fontSize = 13.sp), color = Mts.InkMuted) } }
    }
}

@Composable
private fun SubTasks(d: TaskDetail, actions: TasksActions) {
    val subs = d.subtasks
    if (subs.isEmpty() && d.prompt != "sub" && d.task.parentId != null) return
    Label(if (subs.isEmpty()) "Sub-tasks" else "Sub-tasks · ${subs.count { it.status == "complete" }} of ${subs.size}")
    if (subs.isNotEmpty()) {
        val done = subs.count { it.status == "complete" }.toFloat() / subs.size
        Box(Modifier.fillMaxWidth().size(width = 0.dp, height = 4.dp).background(Mts.Surface200, RoundedCornerShape(2.dp))) { Box(Modifier.fillMaxWidth(done).size(width = 0.dp, height = 4.dp).background(Mts.BrandGreen, RoundedCornerShape(2.dp))) }
    }
    subs.forEach { s ->
        val on = s.status == "complete"
        Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(48.dp).clickable(role = Role.Checkbox, onClickLabel = if (on) "Mark not done" else "Mark done") { actions.onToggleSub(s) }, contentAlignment = Alignment.Center) {
                Box(Modifier.size(22.dp).then(if (on) Modifier.background(Mts.BrandGreen, RoundedCornerShape(6.dp)) else Modifier.border(2.dp, Mts.Line, RoundedCornerShape(6.dp))), contentAlignment = Alignment.Center) { if (on) Icon(HqIcons.CheckSmall, null, Modifier.size(16.dp), tint = Mts.OnGreen) }
            }
            Text(s.title, style = HqType.Body, color = if (on) Mts.InkMuted else Mts.Ink, textDecoration = if (on) androidx.compose.ui.text.style.TextDecoration.LineThrough else null)
        }
    }
    if (d.prompt == "sub") Entry("Add a sub-task", d.draft, actions.onDraft, actions.onAddSub)
    else Row(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button) { actions.onPrompt("sub") }, horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(HqIcons.Plus, null, Modifier.size(20.dp), tint = Mts.BrandBlue); Text("Add a sub-task", style = HqType.BodySmBold, color = Mts.BrandBlue)
    }
}

@Composable
private fun Comments(d: TaskDetail, actions: TasksActions) {
    if (d.comments.isNotEmpty()) Label("Comments")
    d.comments.forEach { c ->
        Column { Text(c.author.ifBlank { "Someone" }, style = HqType.BodySmBold, color = Mts.Ink); Text(c.body, style = HqType.BodySm, color = Mts.InkMuted) }
    }
    if (d.prompt == null) Entry("Add a comment", d.draft, actions.onDraft, actions.onComment)
}

@Composable
private fun Entry(hint: String, value: String, onChange: (String) -> Unit, onSend: () -> Unit) {
    val shape = RoundedCornerShape(24.dp)
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        BasicTextField(
            value, onChange, Modifier.weight(1f).heightIn(min = 48.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp, vertical = 11.dp),
            textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
            decorationBox = { inner -> Box { if (value.isEmpty()) Text(hint, style = HqType.Body, color = Mts.InkMuted); inner() } },
        )
        Box(Modifier.size(48.dp).clip(CircleShape).background(if (value.isBlank()) Mts.Surface200 else Mts.BrandGreen).alpha(if (value.isBlank()) 0.5f else 1f).clickable(enabled = value.isNotBlank(), role = Role.Button, onClickLabel = "Send", onClick = onSend), contentAlignment = Alignment.Center) {
            Icon(HqIcons.Send, null, Modifier.size(24.dp), tint = if (value.isBlank()) Mts.InkMuted else Mts.OnGreen)
        }
    }
}

@Composable
private fun SendBackDialog(d: TaskDetail, actions: TasksActions) {
    BackHandler { actions.onPrompt(null) }
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.62f)).clickable(onClick = { actions.onPrompt(null) }))
        Column(Modifier.padding(start = 24.dp, top = 160.dp, end = 24.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("What should change?", style = HqType.H2, color = Mts.Ink)
            Text("Your answer goes on the task as a comment, and it moves back to In progress.", style = HqType.BodySm, color = Mts.InkMuted)
            val shape = RoundedCornerShape(12.dp)
            BasicTextField(d.draft, actions.onDraft, Modifier.fillMaxWidth().heightIn(min = 96.dp).clip(shape).background(Mts.Surface000).border(1.dp, Mts.Line, shape).padding(12.dp), textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink))
            Box(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).alpha(if (d.draft.isBlank()) 0.5f else 1f).clickable(enabled = d.draft.isNotBlank(), role = Role.Button, onClick = actions.onSendBack), contentAlignment = Alignment.Center) { Text("Send back", style = HqType.BodySmBold, color = Mts.OnGreen) }
        }
    }
}
