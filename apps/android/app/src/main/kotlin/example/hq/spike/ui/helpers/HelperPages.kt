package example.hq.spike.ui.helpers

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.chat.ActionRow
import example.hq.spike.ui.chat.SheetFrame
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.inbox.PrimaryButton
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private val Mono = HqType.Caption.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp, lineHeight = 17.sp)
private const val RAW_LINES = 12
private val QUICK_REPLIES = listOf("Keep going" to "Please keep going.", "Explain what you did" to "Please explain what you did, in plain words.")

/** What is drawn over the whole shell for the open helper: its page, the question it is asking, or its terminal. */
@Composable
fun HelperOverlays(ui: HelpersUi, actions: HelpersActions) {
    val open = ui.open
    val card = ui.openCard
    if (open != null && card != null) {
        when {
            open.technical -> TechnicalPage(card, open, actions)
            open.page == HelperPage.Ask && card.ask != null -> AskPage(card, open, actions)
            else -> HelperPageView(card, open, ui, actions)
        }
        if (open.confirmStop) StopDialog(card, actions)
        if (open.rename != null) RenameDialog(open.rename, actions)
        if (open.menu) MenuSheet(card, actions)
    }
    ui.start?.let { StartSheet(it, actions) }
}

@Composable
private fun PageBar(title: String, status: Pair<String, Color>?, onBack: () -> Unit, trailing: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 4.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        IconAction(HqIcons.ArrowBack, "Back", iconSize = 24.dp, onClick = onBack)
        Column(Modifier.weight(1f).padding(start = 4.dp)) {
            Text(title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            status?.let { (text, color) ->
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Box(Modifier.size(8.dp).background(color, CircleShape))
                    Text(text, style = HqType.Caption, color = Mts.InkMuted, maxLines = 1)
                }
            }
        }
        trailing()
    }
}

private fun statusOf(card: HelperCard): Pair<String, Color> {
    val color = when (card.state) {
        HelperState.Working -> Mts.BrandBlue
        HelperState.NeedsOk, HelperState.Offline, HelperState.Lost -> Mts.Amber
        else -> Mts.Line
    }
    return listOf(HelperWords.stateWords(card.state), card.time).filter { it.isNotBlank() }.joinToString(" · ") to color
}

private fun Modifier.page() = fillMaxSize().background(Mts.Surface000).statusBarsPadding().imePadding().navigationBarsPadding()

@Composable
private fun Caps(text: String) = Text(text.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted)

// ---------------------------------------------------------------- one helper

@Composable
private fun HelperPageView(card: HelperCard, open: HelperOpen, ui: HelpersUi, actions: HelpersActions) {
    BackHandler(onBack = actions.onClose)
    val running = card.state != HelperState.Finished && card.state != HelperState.Stopped
    Column(Modifier.page()) {
        PageBar(card.title, statusOf(card), actions.onClose) { IconAction(HqIcons.MoreVertical, "More", iconSize = 24.dp) { actions.onMenu(true) } }
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 8.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            if (card.state == HelperState.NeedsOk && card.ask != null) AskBanner(card, actions.onSeeAsk)
            RightNow(card, open, ui)
            if (running && card.state != HelperState.Offline) {
                QuickReplies(open.sending, actions.onQuick)
                MessageRow(open, actions)
            }
            open.error?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
            TechnicalRow(actions.onTechnical)
        }
    }
}

@Composable
private fun AskBanner(card: HelperCard, onOpen: () -> Unit) {
    val shape = RoundedCornerShape(16.dp)
    Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Amber, shape).clickable(role = Role.Button, onClick = onOpen).padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Needs your OK", style = HqType.BodyBold, color = Mts.Amber)
        Text(card.line, style = HqType.BodySm, color = Mts.Ink)
    }
}

@Composable
private fun RightNow(card: HelperCard, open: HelperOpen, ui: HelpersUi) {
    val shape = RoundedCornerShape(16.dp)
    val words = when (card.state) {
        HelperState.Working -> card.line
        HelperState.Ready, HelperState.NeedsOk -> HelperWords.lastWords(open.screen) ?: card.line
        else -> card.line
    }
    Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Caps(if (card.state == HelperState.Finished || card.state == HelperState.Stopped) "Last seen" else "Right now")
        Text(words, style = HqType.BodyBold.copy(fontSize = 18.sp, lineHeight = 26.sp), color = Mts.Ink, maxLines = 4, overflow = TextOverflow.Ellipsis)
        Text("${card.agent} · on ${card.where}", style = SmallText, color = Mts.InkMuted)
    }
    if (card.state == HelperState.Offline) Text("The computer did not answer. Its helper may still be working there.", style = HqType.BodySm, color = Mts.InkMuted)
}

@Composable
private fun QuickReplies(sending: Boolean, onQuick: (String) -> Unit) {
    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        QUICK_REPLIES.forEach { (label, text) ->
            Box(
                Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(enabled = !sending, role = Role.Button, onClickLabel = label) { onQuick(text) },
                contentAlignment = Alignment.Center,
            ) {
                Box(Modifier.heightIn(min = 40.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).padding(horizontal = 14.dp), contentAlignment = Alignment.Center) {
                    Text(label, style = HqType.BodySmBold, color = if (sending) Mts.InkMuted else Mts.Ink, maxLines = 1)
                }
            }
        }
    }
}

@Composable
private fun MessageRow(open: HelperOpen, actions: HelpersActions) {
    val shape = RoundedCornerShape(24.dp)
    val ready = open.draft.isNotBlank() && !open.sending
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        BasicTextField(
            open.draft, actions.onDraft, Modifier.weight(1f).heightIn(min = 48.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp, vertical = 11.dp),
            textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
            decorationBox = { inner -> Box { if (open.draft.isEmpty()) Text("Tell this helper something", style = HqType.Body, color = Mts.InkMuted); inner() } },
        )
        Box(
            Modifier.size(48.dp).clip(CircleShape).background(if (ready) Mts.BrandGreen else Mts.Surface200).alpha(if (ready) 1f else 0.5f).clickable(enabled = ready, role = Role.Button, onClickLabel = "Send", onClick = actions.onSend),
            contentAlignment = Alignment.Center,
        ) { Icon(HqIcons.Send, null, Modifier.size(24.dp), tint = if (ready) Mts.OnGreen else Mts.InkMuted) }
    }
}

@Composable
private fun TechnicalRow(onOpen: (Boolean) -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(shape).background(Mts.Surface100).clickable(role = Role.Button) { onOpen(true) }.padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.Code, null, Modifier.size(22.dp), tint = Mts.Ink)
        Text("Technical details", style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
        Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

// ---------------------------------------------------------------- needs your OK

@Composable
private fun AskPage(card: HelperCard, open: HelperOpen, actions: HelpersActions) {
    BackHandler(onBack = actions.onClose)
    val ask = card.ask ?: return
    Column(Modifier.page()) {
        PageBar("Needs your OK", null, actions.onClose)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 8.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(40.dp).background(Mts.Surface200, CircleShape), contentAlignment = Alignment.Center) { Icon(HqIcons.Code, null, Modifier.size(22.dp), tint = Mts.Ink) }
                Column {
                    Text(card.agent, style = HqType.BodyBold, color = Mts.Ink)
                    Text("on ${card.where} · job: ${card.title}", style = SmallText, color = Mts.InkMuted, maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
            }
            Text(HelperWords.askTitle(card.agent, ask), style = HqType.H2.copy(fontSize = 26.sp, lineHeight = 32.sp), color = Mts.Ink)
            val shape = RoundedCornerShape(16.dp)
            if (ask.subject != null) Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Caps("File")
                Text(ask.subject, style = HqType.BodyBold.copy(fontFamily = FontFamily.Monospace), color = Mts.Ink)
            } else Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Caps(if (ask.kind == AskKind.Command) "Command" else "It asked")
                ask.detail?.let { Text(it, style = HqType.BodyBold.copy(fontFamily = FontFamily.Monospace, fontSize = 14.sp, lineHeight = 20.sp), color = Mts.Ink) }
                Text(ask.question, style = HqType.BodySm, color = Mts.InkMuted)
            }
            Outcome(HqIcons.Check, Mts.BrandBlue, "If you allow it", HelperWords.ifAllowed(ask, card.where))
            Outcome(HqIcons.Pause, Mts.InkMuted, "If you say not now", HelperWords.IF_NOT_NOW)
            open.error?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
            Row(Modifier.padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PrimaryButton(if (open.answering) "Answering…" else "Allow once", Modifier.weight(1f), enabled = !open.answering) { actions.onAllow(card.sessionId) }
                OutlineButton("Not now", Modifier.weight(1f), enabled = !open.answering) { actions.onNotNow(card.sessionId) }
            }
            Row(
                Modifier.heightIn(min = 48.dp).clip(RoundedCornerShape(12.dp)).clickable(role = Role.Button, onClick = actions.onToggleRaw).padding(end = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Text("See exactly what it asked", style = HqType.BodySmBold, color = Mts.InkMuted)
                Icon(if (open.raw) HqIcons.ChevronUp else HqIcons.ChevronDown, null, Modifier.size(18.dp), tint = Mts.InkMuted)
            }
            if (open.raw) Terminal(HelperWords.rawTail(open.screen, RAW_LINES))
            Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onSeeHelper), contentAlignment = Alignment.CenterStart) {
                Text("Open the helper instead", style = HqType.BodySmBold, color = Mts.BrandBlue)
            }
        }
    }
}

@Composable
private fun Outcome(icon: androidx.compose.ui.graphics.vector.ImageVector, tint: Color, head: String, body: String) {
    val shape = RoundedCornerShape(12.dp)
    Row(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Icon(icon, null, Modifier.size(22.dp), tint = tint)
        Column { Text(head, style = HqType.BodySmBold, color = Mts.Ink); Text(body, style = HqType.BodySm, color = Mts.InkMuted) }
    }
}

// ---------------------------------------------------------------- technical details

@Composable
private fun Terminal(lines: List<String>) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface000).border(1.dp, Mts.Hairline, shape).padding(horizontal = 12.dp, vertical = 10.dp)) {
        if (lines.isEmpty()) Text("Nothing on its screen yet.", style = Mono, color = Mts.InkMuted)
        lines.forEach { Text(it, style = Mono, color = Mts.Ink) }
    }
}

private val KEYS = listOf("enter" to "enter", "esc" to "esc", "↑" to "up", "↓" to "down", "y" to "y", "n" to "n")

@Composable
private fun TechnicalPage(card: HelperCard, open: HelperOpen, actions: HelpersActions) {
    BackHandler { actions.onTechnical(false) }
    Column(Modifier.page()) {
        PageBar("Technical details", statusOf(card), { actions.onTechnical(false) })
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState(), reverseScrolling = true).padding(start = 16.dp, end = 16.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text("Terminal · read only", style = SmallText, color = Mts.InkMuted)
            Terminal(open.screen.map { HelperWords.cleanLine(it) }.filter { it.isNotEmpty() }.takeLast(TECH_LINES))
        }
        open.error?.let { Text(it, style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.padding(horizontal = 16.dp)) }
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            KEYS.forEach { (label, key) ->
                Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(role = Role.Button, onClickLabel = "Send $label") { actions.onKey(key) }, contentAlignment = Alignment.Center) {
                    Box(Modifier.heightIn(min = 40.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).padding(horizontal = 14.dp), contentAlignment = Alignment.Center) {
                        Text(label, style = HqType.BodySmBold.copy(fontFamily = FontFamily.Monospace, fontSize = 13.sp), color = Mts.Ink, maxLines = 1)
                    }
                }
            }
        }
        Box(Modifier.padding(start = 8.dp, end = 8.dp, bottom = 8.dp)) { MessageRow(open, actions) }
    }
}

private const val TECH_LINES = 60

// ---------------------------------------------------------------- dialogs and sheets

@Composable
private fun StopDialog(card: HelperCard, actions: HelpersActions) {
    BackHandler { actions.onAskStop(false) }
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.62f)).clickable(onClick = { actions.onAskStop(false) }))
        Column(Modifier.padding(start = 24.dp, top = 220.dp, end = 24.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Stop this helper?", style = HqType.H2, color = Mts.Ink)
            Text("${card.agent} stops what it is doing and its terminal on ${card.where} closes. Nothing it already saved is undone.", style = HqType.BodySm, color = Mts.InkMuted)
            PrimaryButton("Yes, stop it", Modifier.fillMaxWidth(), onClick = actions.onStop)
            OutlineButton("Keep going", Modifier.fillMaxWidth()) { actions.onAskStop(false) }
        }
    }
}

@Composable
private fun RenameDialog(text: String, actions: HelpersActions) {
    BackHandler { actions.onRename(null) }
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.62f)).clickable(onClick = { actions.onRename(null) }))
        Column(Modifier.padding(start = 24.dp, top = 220.dp, end = 24.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Rename this helper", style = HqType.H2, color = Mts.Ink)
            val shape = RoundedCornerShape(12.dp)
            BasicTextField(text, { actions.onRename(it) }, Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(shape).background(Mts.Surface000).border(1.dp, Mts.Line, shape).padding(12.dp), textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink))
            PrimaryButton("Save", Modifier.fillMaxWidth(), enabled = text.isNotBlank(), onClick = actions.onSaveRename)
        }
    }
}

@Composable
private fun MenuSheet(card: HelperCard, actions: HelpersActions) {
    val running = card.state != HelperState.Finished && card.state != HelperState.Stopped
    SheetFrame({ actions.onMenu(false) }, top = null) {
        ActionRow(HqIcons.Edit, "Rename", null, { actions.onRename(card.title) })
        if (running) ActionRow(HqIcons.Pause, "Stop", "It stops and its terminal closes. Asks first.", { actions.onAskStop(true) }, tint = Mts.DangerText)
    }
}

@Composable
private fun StartSheet(form: StartForm, actions: HelpersActions) {
    SheetFrame(actions.onStartClose, top = 90.dp) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Start a helper", style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                IconAction(HqIcons.Close, "Close", iconSize = 24.dp, onClick = actions.onStartClose)
            }
            val shape = RoundedCornerShape(12.dp)
            Column(Modifier.fillMaxWidth().heightIn(min = 96.dp).clip(shape).background(Mts.Surface000).border(2.dp, Mts.BrandBlue, shape).padding(horizontal = 16.dp, vertical = 10.dp)) {
                Text("What should it do?", style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
                BasicTextField(form.job, actions.onJob, Modifier.fillMaxWidth(), textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink))
            }
            Caps("Which helper?")
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("claude-code" to "Claude", "codex" to "Codex").forEach { (key, label) ->
                    val on = form.harness == key
                    Box(
                        Modifier.weight(1f).heightIn(min = 56.dp).clip(shape).background(Mts.Surface100).border(1.dp, if (on) Mts.BrandGreen else Mts.Line, shape)
                            .clickable(role = Role.RadioButton, onClickLabel = label) { actions.onHarness(key) }.padding(horizontal = 16.dp, vertical = 12.dp),
                        contentAlignment = Alignment.CenterStart,
                    ) { Text(label, style = HqType.BodyBold, color = Mts.Ink) }
                }
            }
            Row(
                Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(shape).clickable(role = Role.Button, onClickLabel = "Change computer", onClick = actions.onNextHost).padding(horizontal = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text("Where", style = HqType.BodyBold, color = Mts.Ink)
                    Text(whereWords(form), style = HqType.BodySm, color = if (form.host == null && form.hosts != null) Mts.Amber else Mts.InkMuted)
                }
                if ((form.hosts.orEmpty().count { it.reachable }) > 1) Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
            Text("It asks before it changes anything. You will see its questions here and in your Inbox.", style = SmallText, color = Mts.InkMuted)
            form.error?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
            PrimaryButton(if (form.busy) "Starting…" else "Start", Modifier.fillMaxWidth(), enabled = form.job.isNotBlank() && form.host != null && !form.busy, onClick = actions.onStartSubmit)
        }
    }
}

private fun whereWords(f: StartForm): String = when {
    f.hosts == null -> "Looking for your computers…"
    f.host != null -> HelperWords.placeName(f.host).replaceFirstChar(Char::uppercase)
    f.hosts.isEmpty() -> "HQ could not list your computers."
    else -> f.hosts.first().let { HelperWords.hostProblem(it.name, it.error) }
}
