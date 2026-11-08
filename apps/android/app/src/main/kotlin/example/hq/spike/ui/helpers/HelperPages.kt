package example.hq.spike.ui.helpers

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.LifecycleResumeEffect
import example.hq.spike.R
import example.hq.spike.ui.chat.ActionRow
import example.hq.spike.ui.chat.SheetFrame
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.inbox.PrimaryButton
import example.hq.spike.ui.pairing.PairingActivity
import example.hq.spike.ui.shell.ConfirmCopy
import example.hq.spike.ui.shell.ConfirmDialog
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.consumeTouches
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private val Mono = HqType.Caption.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp, lineHeight = 17.sp)
private const val RAW_LINES = 12

/** What is drawn over the whole shell for the open helper: its page, the question it is asking, or its terminal. */
@Composable
fun HelperOverlays(shownUi: HelpersUi, actions: HelpersActions) {
    val ui = shownUi.copy(words = example.hq.spike.ui.shell.rememberWords())
    val open = ui.open
    val card = ui.openCard
    if (open != null && card != null) {
        when {
            open.technical -> TechnicalPage(card, open, actions)
            open.page == HelperPage.Ask && card.ask != null -> AskPage(card, open, actions)
            else -> HelperPageView(card, open, ui, actions)
        }
        if (open.confirmStop) StopDialog(card, open, actions)
        if (open.rename != null) RenameDialog(open.rename, actions)
        if (open.menu) MenuSheet(card, actions)
    }
    ui.start?.let { StartSheet(it, actions) }
}

@Composable
private fun PageBar(title: String, status: Pair<String, Color>?, onBack: () -> Unit, trailing: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 4.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        IconAction(HqIcons.ArrowBack, stringResource(R.string.notify_back), iconSize = 24.dp, onClick = onBack)
        Column(Modifier.weight(1f).padding(start = 4.dp)) {
            Text(title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            status?.let { (text, color) ->
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Box(Modifier.size(8.dp).background(color, CircleShape))
                    Text(text, style = HqType.Caption, color = Mts.InkMuted)
                }
            }
        }
        trailing()
    }
}

private fun statusOf(w: example.hq.spike.ui.shell.Words, card: HelperCard): Pair<String, Color> {
    val color = when (card.state) {
        HelperState.Working -> Mts.BrandBlue
        HelperState.NeedsOk, HelperState.Offline, HelperState.Lost -> Mts.Amber
        else -> Mts.Line
    }
    return listOf(HelperWords.stateWords(w, card.state, card.own), card.time).filter { it.isNotBlank() }.joinToString(" · ") to color
}

private fun Modifier.page() = fillMaxSize().background(Mts.Surface000).consumeTouches().statusBarsPadding().imePadding().navigationBarsPadding()

@Composable
private fun Caps(text: String) = Text(text.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted)

// ---------------------------------------------------------------- one helper

@Composable
private fun HelperPageView(card: HelperCard, open: HelperOpen, ui: HelpersUi, actions: HelpersActions) {
    BackHandler(onBack = actions.onClose)
    val running = card.state != HelperState.Finished && card.state != HelperState.Stopped
    Column(Modifier.page()) {
        PageBar(card.title, statusOf(example.hq.spike.ui.shell.rememberWords(), card), actions.onClose) { IconAction(HqIcons.MoreVertical, stringResource(R.string.tasks_more), iconSize = 24.dp) { actions.onMenu(true) } }
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
        Text(stringResource(R.string.helpers_needs_your_ok), style = HqType.BodyBold, color = Mts.Amber)
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
        Caps(stringResource(if (card.state == HelperState.Finished || card.state == HelperState.Stopped) R.string.helpers_last_seen else R.string.helpers_right_now))
        Text(words, style = HqType.BodyBold.copy(fontSize = 18.sp, lineHeight = 26.sp), color = Mts.Ink, maxLines = 4, overflow = TextOverflow.Ellipsis)
        Text(stringResource(R.string.helpers_agent_where, card.agent, card.where), style = SmallText, color = Mts.InkMuted)
    }
    if (card.state == HelperState.Offline) Text(stringResource(R.string.helpers_the_computer_did_not), style = HqType.BodySm, color = Mts.InkMuted)
}

@Composable
private fun QuickReplies(sending: Boolean, onQuick: (String) -> Unit) {
    Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        listOf(R.string.helpers_quick_keep_going to R.string.helpers_quick_keep_going_text, R.string.helpers_quick_explain to R.string.helpers_quick_explain_text).forEach { (labelId, textId) ->
            val label = stringResource(labelId)
            val text = stringResource(textId)
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
            decorationBox = { inner -> Box { if (open.draft.isEmpty()) Text(stringResource(R.string.helpers_tell_this_helper_something), style = HqType.Body, color = Mts.InkMuted); inner() } },
        )
        Box(
            Modifier.size(48.dp).clip(CircleShape).background(if (ready) Mts.BrandGreen else Mts.Surface200).alpha(if (ready) 1f else 0.5f).clickable(enabled = ready, role = Role.Button, onClickLabel = stringResource(R.string.chat_send), onClick = actions.onSend),
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
        Text(stringResource(R.string.settings_technical_title), style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
        Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

// ---------------------------------------------------------------- needs your OK

@Composable
private fun AskPage(card: HelperCard, open: HelperOpen, actions: HelpersActions) {
    BackHandler(onBack = actions.onClose)
    val ask = card.ask ?: return
    Column(Modifier.page()) {
        PageBar(stringResource(R.string.helpers_needs_your_ok), null, actions.onClose)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 8.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(40.dp).background(Mts.Surface200, CircleShape), contentAlignment = Alignment.Center) { Icon(HqIcons.Code, null, Modifier.size(22.dp), tint = Mts.Ink) }
                Column {
                    Text(card.agent, style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.helpers_where_job, card.where, card.title), style = SmallText, color = Mts.InkMuted)
                }
            }
            Text(HelperWords.askTitle(example.hq.spike.ui.shell.rememberWords(), card.agent, ask), style = HqType.H2.copy(fontSize = 26.sp, lineHeight = 32.sp), color = Mts.Ink)
            val shape = RoundedCornerShape(16.dp)
            if (ask.subject != null) Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Caps(stringResource(R.string.helpers_file))
                Text(ask.subject, style = HqType.BodyBold.copy(fontFamily = FontFamily.Monospace), color = Mts.Ink)
            } else Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Caps(stringResource(if (ask.kind == AskKind.Command) R.string.helpers_command else R.string.helpers_it_asked))
                ask.detail.forEach { Text(it, style = HqType.BodyBold.copy(fontFamily = FontFamily.Monospace, fontSize = 14.sp, lineHeight = 20.sp), color = Mts.Ink) }
                Text(ask.question, style = HqType.BodySm, color = Mts.InkMuted)
            }
            Outcome(HqIcons.Check, Mts.BrandBlue, stringResource(R.string.helpers_if_you_allow), HelperWords.ifAllowed(example.hq.spike.ui.shell.rememberWords(), ask, card.where))
            Outcome(HqIcons.Pause, Mts.InkMuted, stringResource(R.string.helpers_if_you_say_not_now), HelperWords.ifNotNow(example.hq.spike.ui.shell.rememberWords()))
            open.error?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
            Row(Modifier.padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PrimaryButton(stringResource(if (open.answering) R.string.helpers_answering else R.string.notice_action_allow_once), Modifier.weight(1f), enabled = !open.answering) { actions.onAllow(card.sessionId, ask) }
                OutlineButton(stringResource(R.string.confirm_not_now), Modifier.weight(1f), enabled = !open.answering) { actions.onNotNow(card.sessionId, ask) }
            }
            Row(
                Modifier.heightIn(min = 48.dp).clip(RoundedCornerShape(12.dp)).clickable(role = Role.Button, onClick = actions.onToggleRaw).padding(end = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(stringResource(R.string.helpers_see_exactly_what_it), style = HqType.BodySmBold, color = Mts.InkMuted)
                Icon(if (open.raw) HqIcons.ChevronUp else HqIcons.ChevronDown, null, Modifier.size(18.dp), tint = Mts.InkMuted)
            }
            if (open.raw) Terminal(HelperWords.rawTail(open.screen, RAW_LINES))
            Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onSeeHelper), contentAlignment = Alignment.CenterStart) {
                Text(stringResource(R.string.helpers_open_the_helper_instead), style = HqType.BodySmBold, color = Mts.BrandBlue)
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
        if (lines.isEmpty()) Text(stringResource(R.string.helpers_nothing_on_its_screen), style = Mono, color = Mts.InkMuted)
        lines.forEach { Text(it, style = Mono, color = Mts.Ink) }
    }
}

private val KEYS = listOf("enter" to "enter", "esc" to "esc", "↑" to "up", "↓" to "down", "y" to "y", "n" to "n")

@Composable
private fun TechnicalPage(card: HelperCard, open: HelperOpen, actions: HelpersActions) {
    BackHandler { actions.onTechnical(false) }
    Column(Modifier.page()) {
        PageBar(stringResource(R.string.helpers_technical_details), statusOf(example.hq.spike.ui.shell.rememberWords(), card), { actions.onTechnical(false) })
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState(), reverseScrolling = true).padding(start = 16.dp, end = 16.dp, bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(stringResource(R.string.helpers_terminal_read_only), style = SmallText, color = Mts.InkMuted)
            Terminal(HelperWords.rawTail(open.screen, TECH_LINES))
        }
        open.error?.let { Text(it, style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.padding(horizontal = 16.dp)) }
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            KEYS.forEach { (label, key) ->
                Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(role = Role.Button, onClickLabel = stringResource(R.string.helpers_send_key, label)) { actions.onKey(key) }, contentAlignment = Alignment.Center) {
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
private fun StopDialog(card: HelperCard, open: HelperOpen, actions: HelpersActions) {
    val context = LocalContext.current
    ConfirmDialog(
        ConfirmCopy.stopHelper(context, card.agent), onKeep = { actions.onAskStop(false) }, onAct = actions.onStop,
        busy = open.stopping, error = open.stopError,
    )
}

@Composable
private fun RenameDialog(text: String, actions: HelpersActions) {
    BackHandler { actions.onRename(null) }
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.62f)).clickable(onClick = { actions.onRename(null) }))
        Column(Modifier.padding(start = 24.dp, top = 220.dp, end = 24.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.helpers_rename_this_helper), style = HqType.H2, color = Mts.Ink)
            val shape = RoundedCornerShape(12.dp)
            BasicTextField(text, { actions.onRename(it) }, Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(shape).background(Mts.Surface000).border(1.dp, Mts.Line, shape).padding(12.dp), textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink), singleLine = true)
            PrimaryButton(stringResource(R.string.quick_note_save), Modifier.fillMaxWidth(), enabled = text.isNotBlank(), onClick = actions.onSaveRename)
        }
    }
}

@Composable
private fun MenuSheet(card: HelperCard, actions: HelpersActions) {
    val running = card.state != HelperState.Finished && card.state != HelperState.Stopped
    SheetFrame({ actions.onMenu(false) }, top = null) {
        ActionRow(HqIcons.Edit, stringResource(R.string.helpers_rename), null, { actions.onRename(card.title) })
        ActionRow(HqIcons.Chat, stringResource(R.string.helpers_follow_in_chat), stringResource(R.string.helpers_follow_in_chat_hint), actions.onFollow)
        if (running) ActionRow(HqIcons.Pause, stringResource(R.string.chat_stop), stringResource(R.string.helpers_stop_hint), { actions.onAskStop(true) }, tint = Mts.Ink)
    }
}

@Composable
private fun StartSheet(form: StartForm, actions: HelpersActions) {
    SheetFrame(actions.onStartClose, top = 90.dp) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.helpers_start_a_helper), style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                IconAction(HqIcons.Close, stringResource(R.string.close), iconSize = 24.dp, onClick = actions.onStartClose)
            }
            val shape = RoundedCornerShape(12.dp)
            Column(Modifier.fillMaxWidth().heightIn(min = 96.dp).clip(shape).background(Mts.Surface000).border(2.dp, Mts.BrandBlue, shape).padding(horizontal = 16.dp, vertical = 10.dp)) {
                Text(stringResource(R.string.helpers_what_should_it_do), style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
                BasicTextField(form.job, actions.onJob, Modifier.fillMaxWidth(), textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink))
            }
            Caps(stringResource(R.string.helpers_which_helper))
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
            if (HelperWords.needsComputer(form.hosts)) PairComputer(actions) else StartWhere(form, shape, actions)
        }
    }
}

@Composable
private fun StartWhere(form: StartForm, shape: RoundedCornerShape, actions: HelpersActions) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(shape).clickable(role = Role.Button, onClickLabel = stringResource(R.string.helpers_change_computer), onClick = actions.onNextHost).padding(horizontal = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(stringResource(R.string.helpers_where), style = HqType.BodyBold, color = Mts.Ink)
            Text(whereWords(example.hq.spike.ui.shell.rememberWords(), form), style = HqType.BodySm, color = if (form.host == null && form.hosts != null) Mts.Amber else Mts.InkMuted)
        }
        if ((form.hosts.orEmpty().count { it.reachable }) > 1) Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
    Text(stringResource(R.string.helpers_it_asks_before_it), style = SmallText, color = Mts.InkMuted)
    form.error?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
    PrimaryButton(stringResource(if (form.busy) R.string.helpers_starting else R.string.helpers_start), Modifier.fillMaxWidth(), enabled = form.job.isNotBlank() && form.host != null && !form.busy, onClick = actions.onStartSubmit)
}

/**
 * The start sheet with no computer to run on: the one line to run on the computer (copied, or sent to it from this phone), then
 * the scanner for the code that line shows. Coming back to the sheet asks HQ again, so a computer paired meanwhile shows up.
 */
@Composable
private fun PairComputer(actions: HelpersActions) {
    val context = LocalContext.current
    var copied by remember { mutableStateOf(false) }
    LifecycleResumeEffect(Unit) {
        actions.onRefreshHosts()
        onPauseOrDispose {}
    }
    val shape = RoundedCornerShape(12.dp)
    Text(stringResource(R.string.helpers_pair_title), style = HqType.BodyBold, color = Mts.Ink)
    Text(stringResource(R.string.helpers_pair_body), style = HqType.BodySm, color = Mts.InkMuted)
    Text(
        HelperWords.PAIR_COMMAND, style = HqType.BodySm.copy(fontFamily = FontFamily.Monospace), color = Mts.Ink,
        modifier = Modifier.fillMaxWidth().clip(shape).background(Mts.Surface000).border(1.dp, Mts.Line, shape).padding(12.dp),
    )
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        PairAction(stringResource(if (copied) R.string.helpers_pair_copied else R.string.helpers_pair_copy), Modifier.weight(1f)) {
            context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("hq", HelperWords.PAIR_COMMAND))
            copied = true
        }
        PairAction(stringResource(R.string.helpers_pair_send), Modifier.weight(1f)) {
            val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, HelperWords.PAIR_COMMAND)
            context.startActivity(Intent.createChooser(send, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }
    Text(stringResource(R.string.helpers_pair_then), style = HqType.BodySm, color = Mts.InkMuted)
    PrimaryButton(stringResource(R.string.helpers_pair_scan), Modifier.fillMaxWidth()) {
        context.startActivity(Intent(context, PairingActivity::class.java).putExtra(PairingActivity.EXTRA_SCREEN, "scan"))
    }
    Text(stringResource(R.string.helpers_pair_needs), style = SmallText, color = Mts.InkMuted)
}

@Composable
private fun PairAction(label: String, modifier: Modifier, onClick: () -> Unit) {
    Box(
        modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = Mts.Ink, maxLines = 1) }
}

private fun whereWords(w: example.hq.spike.ui.shell.Words, f: StartForm): String = when {
    f.hosts == null -> w.s(R.string.helpers_looking_computers)
    f.host != null -> HelperWords.placeName(w, f.host).replaceFirstChar(Char::uppercase)
    f.hosts.isEmpty() -> w.s(R.string.helpers_could_not_list)
    else -> HelperWords.noComputer(w, f.hosts)
}
