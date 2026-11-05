package example.hq.spike.ui.settings

import example.hq.spike.R
import example.hq.spike.ui.firstrun.FirstRunWords
import example.hq.spike.ui.firstrun.StoredKeysPage
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import java.time.Instant
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.Lifecycle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.runtime.produceState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.measure.Counters
import example.hq.spike.measure.CellWords
import example.hq.spike.measure.Noticed
import example.hq.spike.measure.NoticeLine
import example.hq.spike.update.UpdateLogic
import example.hq.spike.update.UpdateState
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.shell.ConfirmCopy
import example.hq.spike.ui.shell.ConfirmDialog
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.UndoSnackbar
import example.hq.spike.ui.shell.consumeTouches
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.webhost.LocalOpenWeb
import example.hq.spike.AlertPoster
import example.hq.spike.LocalNotif
import example.hq.spike.NotifOrigin
import example.hq.spike.ui.webhost.WebPage
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.delay

private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private const val LINE_MS = 4_000L
private const val REFRESH_MS = 15_000L
private const val PAIR_POLL_MS = 3_000L
private val BAR_HEIGHT = 8.dp
private const val DISABLED_ALPHA = 0.5f
private const val SCRIM_ALPHA = 0.62f
private val VALUE_MAX = 180.dp
private val KEY_COLUMN = 120.dp

private fun icon(key: String): ImageVector = when (key) {
    "background" -> HqIcons.Power
    "battery" -> HqIcons.Battery
    "notifications" -> HqIcons.Inbox
    "capture" -> HqIcons.Plus
    "key" -> HqIcons.Key
    "chat" -> HqIcons.Chat
    "phone" -> HqIcons.Phone
    "shield" -> HqIcons.Shield
    "gauge" -> HqIcons.Gauge
    else -> HqIcons.Archive
}

private fun pageTitle(ui: SettingsUi): String = when (ui.page) {
    SettingsPage.SetupApp -> "Set up ${SettingsWords.appName(ui.setup?.platform ?: "telegram")}"
    SettingsPage.PairApp -> "Pair your ${SettingsWords.appName(ui.pair?.platform ?: "telegram")}"
    else -> title(ui.page)
}

private fun title(page: SettingsPage) = when (page) {
    SettingsPage.Home -> "Settings"
    SettingsPage.Safety -> "Safety and limits"
    SettingsPage.Keys -> "AI service and keys"
    SettingsPage.Messaging -> "Messaging apps"
    SettingsPage.Devices -> "Other devices"
    SettingsPage.Spending -> "Spending limits"
    SettingsPage.About -> "Backup and about"
    SettingsPage.Background -> "Background work"
    SettingsPage.KeepRunning -> "Keep running"
    SettingsPage.Notifications -> "Notifications"
    SettingsPage.Origins -> "Allowed addresses"
    SettingsPage.ConnectedTools -> "Connected tools"
    SettingsPage.Computers -> "Computers"
    SettingsPage.CaptureWidgets -> "Capture and widgets"
    SettingsPage.SetupApp -> "Set up"
    SettingsPage.PairApp -> "Pair"
    SettingsPage.StoredKeys -> FirstRunWords.KEYS_TITLE
    SettingsPage.Technical -> "Technical details"
    SettingsPage.Noticed -> "What HQ noticed"
}

/** Settings over the whole shell, with its own pages: Back goes up one page, then closes. */
@Composable
fun SettingsHost(ui: SettingsUi, actions: SettingsActions) {
    val owner = LocalLifecycleOwner.current
    // One loop for as long as Settings is open and the app is in front: closing Settings ends it, and opening it again starts one.
    LaunchedEffect(ui.open) {
        if (ui.open) owner.lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) { actions.onRefresh(); delay(REFRESH_MS) }
        }
    }
    if (!ui.open) return
    BackHandler(onBack = actions.onBack)
    Box(Modifier.fillMaxSize().background(Mts.Surface000).consumeTouches().statusBarsPadding().navigationBarsPadding()) {
        Column(Modifier.fillMaxSize()) {
            Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 4.dp, end = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                IconAction(HqIcons.ArrowBack, "Back", iconSize = 24.dp, onClick = actions.onBack)
                Text(pageTitle(ui), style = HqType.ScreenTitle, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 4.dp))
            }
            when (ui.page) {
                SettingsPage.Home -> Home(ui, actions)
                SettingsPage.Safety -> Safety(ui, actions)
                SettingsPage.Keys -> Keys(ui, actions)
                SettingsPage.Messaging -> Messaging(ui, actions)
                SettingsPage.Devices -> Devices(ui, actions)
                SettingsPage.Spending -> Spending(ui)
                SettingsPage.About -> About(ui, actions)
                SettingsPage.Background -> Background(ui, actions)
                SettingsPage.KeepRunning -> KeepRunning(ui, actions)
                SettingsPage.Notifications -> Notifications(ui, actions)
                SettingsPage.CaptureWidgets -> CaptureWidgets(ui, actions)
                SettingsPage.SetupApp -> SetupApp(ui, actions)
                SettingsPage.PairApp -> PairApp(ui, actions)
                SettingsPage.StoredKeys -> StoredKeysPage()
                SettingsPage.Technical -> Technical(ui, actions)
                SettingsPage.Noticed -> NoticedPage(Counters.get(androidx.compose.ui.platform.LocalContext.current))
                SettingsPage.Origins -> ReadOnlyList(ui, "No address is listed.") { SettingsWords.originRows(ui.snap.settings).forEach { MonoRow(it) } }
                SettingsPage.ConnectedTools -> ReadOnlyList(ui, "No tool is connected.") { Facts(SettingsWords.toolRows(ui.snap.settings)) }
                SettingsPage.Computers -> ReadOnlyList(ui, "No computer is listed.") { Facts(SettingsWords.computerRows(ui.snap.settings)) }
            }
        }
        val context = androidx.compose.ui.platform.LocalContext.current
        ui.askMode?.let { toActive ->
            val w = SettingsWords.modeDialog(toActive)
            ConfirmDialog(if (toActive) ConfirmCopy.start(context, w.title, w.body) else ConfirmCopy.pause(context, w.title, w.body), actions.onModeCancel, actions.onModeConfirm)
        }
        ui.keySheet?.let { KeySheetView(it, actions) }
        ui.askDisconnect?.let { p -> ConfirmDialog(ConfirmCopy.disconnectApp(context, SettingsWords.appName(p)), { actions.onAskDisconnect(null) }, actions.onDisconnect) }
        ui.askReplace?.let { p -> ConfirmDialog(ConfirmCopy.unlinkAccount(context, SettingsWords.appName(p)), { actions.onAskReplace(null) }, { actions.onPair(p, true) }) }
        if (ui.askSignOutAll) ConfirmDialog(ConfirmCopy.signOutAll(context), { actions.onSignOutAll(false) }, actions.onSignOutAllConfirm)
        ui.askSignOut?.let { d -> ConfirmDialog(ConfirmCopy.signOutDevice(context, d.label), { actions.onAskSignOut(null) }, actions.onSignOutConfirm, busy = ui.busy == "out:${d.id}", error = ui.signOutError) }
        if (ui.askServerDisconnect) ConfirmDialog(ConfirmCopy.disconnectServer(context, ConfirmCopy.hostOf(ui.notif.server.address)), { actions.onAskServerDisconnect(false) }, actions.onServerDisconnect)
        if (ui.askRestart) {
            val w = SettingsWords.RESTART_WORDS
            ConfirmDialog(ConfirmCopy.restart(context, w.title, w.body), { actions.onRestart(false) }, actions.onRestartConfirm)
        }
        ui.undoLine?.let { UndoSnackbar(it, actions.onUndoDiscard, Modifier.align(Alignment.BottomCenter)) }
        ui.line?.let { Snack(it, actions.onDismissLine, Modifier.align(Alignment.BottomCenter)) }
    }
}

@Composable
private fun Snack(text: String, onDismiss: () -> Unit, modifier: Modifier) {
    LaunchedEffect(text) { delay(LINE_MS); onDismiss() }
    Row(modifier.padding(8.dp).fillMaxWidth().heightIn(min = 48.dp).background(Mts.Ink, RoundedCornerShape(6.dp)).padding(horizontal = 16.dp).semantics { liveRegion = LiveRegionMode.Polite }, verticalAlignment = Alignment.CenterVertically) {
        Text(text, style = HqType.BodySm, color = Mts.Surface100)
    }
}

/** A row that opens one of the app's own full screens (pairing). */
@Composable
private fun OpenScreen(text: String, screen: Class<*>) {
    val context = androidx.compose.ui.platform.LocalContext.current
    Text(text, style = HqType.BodyBold, color = Mts.BrandBlue, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable { context.startActivity(android.content.Intent(context, screen)) }.padding(horizontal = 20.dp, vertical = 16.dp))
}

@Composable
private fun Page(content: @Composable () -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) { content() }
}

@Composable
private fun Label(text: String) = Text(text.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 20.dp, end = 16.dp, bottom = 4.dp))

@Composable
private fun Group(content: @Composable () -> Unit) {
    Column(Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Mts.Surface100)) { content() }
}

@Composable
private fun Divider() = Box(Modifier.fillMaxWidth().size(0.dp, 1.dp).background(Mts.Hairline))

@Composable
private fun Notice(text: String, icon: ImageVector = HqIcons.Lock, warn: Boolean = false) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, if (warn) Mts.Amber else Mts.Hairline, shape).padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top,
    ) {
        Icon(icon, null, Modifier.size(22.dp), tint = if (warn) Mts.Amber else Mts.InkMuted)
        Text(text, style = HqType.BodySm, color = Mts.Ink)
    }
}

@Composable
private fun NavRow(row: HomeRow, first: Boolean, onClick: () -> Unit) {
    if (!first) Divider()
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon(row.key), null, Modifier.size(24.dp), tint = Mts.Ink)
        Column(Modifier.weight(1f)) {
            Text(row.title, style = HqType.BodyBold, color = Mts.Ink)
            Text(row.status, style = HqType.BodySm, color = if (row.warn) Mts.Amber else Mts.InkMuted)
        }
        if (row.warn) Icon(HqIcons.Warning, null, Modifier.size(20.dp), tint = Mts.Amber)
        Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

@Composable
private fun Fact(row: FactRow, first: Boolean, onOpen: (SettingsPage) -> Unit = {}) {
    if (!first) Divider()
    val tap = row.opens?.let { page -> Modifier.heightIn(min = 48.dp).clickable(role = Role.Button) { onOpen(page) } } ?: Modifier
    Column(Modifier.fillMaxWidth().then(tap).padding(horizontal = 16.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(row.title, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
            Text(row.value, style = HqType.BodySm, color = if (row.good) Mts.BrandGreen else Mts.InkMuted, textAlign = TextAlign.End, modifier = Modifier.widthIn(max = VALUE_MAX))
            if (row.opens != null) Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
        }
        row.text?.let { Text(it, style = SmallText, color = Mts.InkMuted) }
    }
}

@Composable
private fun Facts(rows: List<FactRow>, onOpen: (SettingsPage) -> Unit = {}) = Group { rows.forEachIndexed { i, r -> Fact(r, i == 0, onOpen) } }

@Composable
private fun Waiting(ui: SettingsUi) {
    if (!ui.loaded) Text(stringResource(R.string.settings_asking_hq), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
}

/** True when the page has nothing to show yet: it says it is asking, or why there is nothing (no answer, or an error answer). */
@Composable
private fun Unanswered(ui: SettingsUi, route: String, have: Boolean): Boolean {
    if (have) return false
    when {
        !ui.loaded -> Waiting(ui)
        route in ui.snap.unavailable -> Notice("HQ answered, but this page is not available here. It may be switched off in this edition or setup.", HqIcons.Info)
        else -> NoAnswer()
    }
    return true
}

@Composable
private fun NoAnswer(text: String = "HQ is not running, so there is nothing to show yet. Try again when it is back.") =
    Notice(text, HqIcons.WifiOff, warn = true)

// ---------------------------------------------------------------- home

@Composable
private fun Home(ui: SettingsUi, actions: SettingsActions) {
    Page {
        SettingsWords.home(ui.snap, ui.now, ui.phone, ui.loaded, ui.backup).forEachIndexed { gi, g ->
            Text(g.title.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = if (gi == 0) 8.dp else 20.dp, end = 16.dp, bottom = 4.dp))
            Group {
                g.rows.forEachIndexed { i, r -> NavRow(r, i == 0) { if (r.page != null) actions.onPage(r.page) else actions.onPhoneRow(r.key) } }
            }
            g.note?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp)) }
        }
    }
}

// ---------------------------------------------------------------- safety and limits

@Composable
private fun Safety(ui: SettingsUi, actions: SettingsActions) {
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SettingsWords.SafetyTab.entries.forEach { t ->
                val on = t == ui.safetyTab
                Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).selectable(selected = on, role = Role.Tab, onClick = { actions.onTab(t) }), contentAlignment = Alignment.Center) {
                    Box(
                        Modifier.heightIn(min = 36.dp).clip(CircleShape).then(if (on) Modifier.background(Mts.BrandGreen, CircleShape).border(1.dp, Mts.BrandGreen, CircleShape) else Modifier.border(1.dp, Mts.Line, CircleShape)).padding(horizontal = 14.dp),
                        contentAlignment = Alignment.Center,
                    ) { Text(t.label, style = HqType.BodySmBold, color = if (on) Mts.OnGreen else Mts.Ink, maxLines = 1) }
                }
            }
        }
        Page {
            Notice("Read only. You can change these in the config file on this phone.")
            val rows = SettingsWords.safetyRows(ui.snap.settings, ui.safetyTab)
            Box(Modifier.padding(top = 12.dp)) { if (rows.isEmpty()) Unanswered(ui, "settings", false) else Facts(rows, actions.onPage) }
        }
    }
}

// ---------------------------------------------------------------- AI service and keys

@Composable
private fun Keys(ui: SettingsUi, actions: SettingsActions) {
    val s = ui.snap.settings
    Page {
        if (Unanswered(ui, "settings", s != null) || s == null) return@Page
        Label("Model")
        Facts(listOf(FactRow("Which AI HQ uses", SettingsWords.activeModel(s)), FactRow("Only AI that runs on this phone", SettingsWords.onOff(s.optJSONObject("model")?.optBoolean("local_only") == true))))
        Text(stringResource(R.string.settings_the_model_is_set), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
        val ai = SettingsWords.aiRows(ui.snap)
        if (ai.isNotEmpty()) {
            Label("Which AI answers")
            Facts(ai)
            Text(stringResource(R.string.settings_hq_tries_them_in), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
        }
        Label("Your keys")
        Group {
            SettingsWords.keyRows(s, ui.snap.copilot, ui.snap.providers).forEachIndexed { i, (name, set, source) ->
                if (i > 0) Divider()
                Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(name, style = HqType.BodyBold, color = Mts.Ink)
                        Text(if (set) (if (source == "env") "Set outside the app" else "Set") else if (source == "unreadable") "Could not be checked" else "Not set", style = HqType.BodySm, color = if (set) Mts.BrandGreen else Mts.InkMuted)
                    }
                    SettingsWords.keyAction(s, name, set, source)?.let { a ->
                        Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).clickable(role = Role.Button) { actions.onKeyOpen(KeySheet(a.id, name, replace = set)) }.padding(horizontal = 16.dp), contentAlignment = Alignment.Center) {
                            Text(a.label, style = HqType.BodySmBold, color = Mts.Ink)
                        }
                    }
                }
            }
        }
        if (ai.isEmpty()) {
            Label("If the first AI is down")
            Group {
                SettingsWords.fallbackRows(s).forEachIndexed { i, (a, b) ->
                    if (i > 0) Divider()
                    Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(a, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
                        Text(b, style = HqType.BodySm, color = Mts.InkMuted)
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- messaging apps

@Composable
private fun Messaging(ui: SettingsUi, actions: SettingsActions) {
    Page {
        if (Unanswered(ui, "chatApps", ui.snap.chatApps != null)) return@Page
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            listOf("telegram" to "Telegram", "discord" to "Discord").forEach { (key, name) -> AppCardView(SettingsWords.appCard(ui.snap.chatApps, key, name, ui.now), key, ui.restarting, actions) }
        }
        Text(if (ui.restarting) "HQ is restarting\u2026" else "Changes restart HQ. This screen reconnects by itself.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
    }
}

@Composable
private fun AppCardView(c: SettingsWords.AppCard, platform: String, busy: Boolean, actions: SettingsActions) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(40.dp).background(Mts.Surface200, CircleShape), contentAlignment = Alignment.Center) { Icon(HqIcons.Chat, null, Modifier.size(22.dp), tint = Mts.Ink) }
            Column {
                Text(c.name, style = HqType.ScreenTitle, color = Mts.Ink)
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(8.dp).background(if (c.line.startsWith("Running")) Mts.BrandGreen else if (c.on) Mts.Amber else Mts.Line, CircleShape))
                    Text(c.line, style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
        }
        if (c.on) {
            c.bot?.let { KeyValue("Bot", it) }
            KeyValue("Your account", c.owner, if (c.owner == "Linked") Mts.BrandGreen else Mts.InkMuted)
            if (c.owner != "Linked") GreenButton("Link my ${c.name}", enabled = !busy) { actions.onPair(platform, false) }
            Row(Modifier.offset(x = (-12).dp), horizontalArrangement = Arrangement.spacedBy(0.dp)) {
                if (c.owner == "Linked") TextLink("Link a different account", Mts.Ink, enabled = !busy) { actions.onAskReplace(platform) }
                TextLink("Disconnect", Mts.Ink, enabled = !busy) { actions.onAskDisconnect(platform) }
            }
        } else {
            Text("Add a bot so HQ can answer in ${c.name}.", style = HqType.BodySm, color = Mts.InkMuted)
            GreenButton("Set up", enabled = !busy) { actions.onSetup(platform) }
        }
    }
}

@Composable
private fun GreenButton(label: String, enabled: Boolean = true, busy: Boolean = false, modifier: Modifier = Modifier.fillMaxWidth(), onClick: () -> Unit) {
    Row(
        modifier.heightIn(min = 48.dp).clip(CircleShape).background(if (enabled) Mts.BrandGreen else Mts.Surface200).clickable(enabled = enabled, role = Role.Button, onClick = onClick),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically,
    ) {
        if (busy) Spinner(18, Mts.OnGreen)
        Text(label, style = HqType.BodySmBold, color = if (enabled) Mts.OnGreen else Mts.InkMuted)
    }
}

/** The Standby warning above the steps and the code: nothing listens while background work is paused. */
@Composable
private fun StandbyNotice(platform: String, onOpen: () -> Unit) {
    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(0.dp)) {
        Notice(SettingsWords.standbyNotice(platform), HqIcons.Warning, warn = true)
        TextLink("Open Background work", Mts.BrandBlue, onClick = onOpen)
    }
}

@Composable
private fun SetupApp(ui: SettingsUi, actions: SettingsActions) {
    val setup = ui.setup ?: return
    val platform = setup.platform
    val context = androidx.compose.ui.platform.LocalContext.current
    val clipboard = androidx.compose.ui.platform.LocalClipboardManager.current
    var typed by remember { androidx.compose.runtime.mutableStateOf("") }
    var okFor by remember { androidx.compose.runtime.mutableStateOf<String?>(null) }
    LaunchedEffect(setup.found) { if (setup.found != null) okFor = typed.trim() }
    val checked = setup.found != null && okFor == typed.trim() && typed.isNotBlank()
    val field = RoundedCornerShape(12.dp)
    Page {
        if (ui.mode != "active") StandbyNotice(platform) { actions.onPage(SettingsPage.Background) }
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            SettingsWords.setupSteps(platform).forEachIndexed { i, text ->
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top) {
                    Box(Modifier.size(28.dp).background(Mts.Surface200, CircleShape), contentAlignment = Alignment.Center) { Text("${i + 1}", style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.Ink) }
                    Text(text, style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(top = 4.dp))
                }
            }
            Box(
                Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button) { runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(SettingsWords.setupUrl(platform))).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)) } }.padding(horizontal = 24.dp),
                contentAlignment = Alignment.Center,
            ) { Text(SettingsWords.setupLabel(platform), style = HqType.BodySmBold, color = Mts.Ink) }
            Row(
                Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(field).background(Mts.Surface100).border(1.dp, Mts.Line, field).padding(start = 16.dp, top = 8.dp, end = 8.dp, bottom = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_bot_token), style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
                    androidx.compose.foundation.text.BasicTextField(
                        typed, { typed = it }, Modifier.fillMaxWidth(), enabled = !setup.saving, singleLine = true,
                        textStyle = HqType.Body.copy(color = Mts.Ink, fontFamily = FontFamily.Monospace, letterSpacing = 0.1.em), cursorBrush = androidx.compose.ui.graphics.SolidColor(Mts.Ink),
                        visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation(),
                        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.Password, autoCorrectEnabled = false),
                    )
                }
                IconAction(HqIcons.Paste, "Paste the token", iconSize = 22.dp) { clipboard.getText()?.text?.let { typed = it.trim() } }
            }
            if (checked) Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(HqIcons.Check, null, Modifier.size(20.dp), tint = Mts.BrandGreen)
                Text("Found your bot: ${setup.found}", style = HqType.BodySmBold, color = Mts.Ink)
            }
            if (checked && setup.intentOff) Text(SettingsWords.INTENT_OFF, style = HqType.BodySm, color = Mts.Amber)
            if (!checked) setup.error?.let { Text(it, style = HqType.BodySm, color = Mts.DangerText) }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Row(
                    Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).clickable(enabled = typed.isNotBlank() && !setup.checking && !setup.saving, role = Role.Button) { actions.onCheckToken(typed) }.padding(horizontal = 24.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (setup.checking) Spinner(18)
                    Text(if (setup.checking) "Checking\u2026" else "Check token", style = HqType.BodySmBold, color = Mts.Ink)
                }
                GreenButton(if (setup.saving) "Restarting\u2026" else "Save and restart", enabled = checked && !setup.saving, busy = setup.saving, modifier = Modifier.weight(1f)) { actions.onSaveToken(typed) }
            }
        }
    }
}

@Composable
private fun PairApp(ui: SettingsUi, actions: SettingsActions) {
    val pair = ui.pair ?: return
    val platform = pair.platform
    val context = androidx.compose.ui.platform.LocalContext.current
    val left by produceState(pair.expiresAt - Instant.now().epochSecond, pair.code) {
        while (true) { value = (pair.expiresAt - Instant.now().epochSecond).coerceAtLeast(0); delay(1_000) }
    }
    LaunchedEffect(pair.code) { while (true) { actions.onPairPoll(); delay(PAIR_POLL_MS) } }
    val expired = left <= 0
    Page {
        if (ui.mode != "active") StandbyNotice(platform) { actions.onPage(SettingsPage.Background) }
        Column(Modifier.fillMaxWidth().padding(start = 16.dp, top = 24.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Text(SettingsWords.pairBlurb(platform), style = HqType.Body, color = Mts.InkMuted, textAlign = TextAlign.Center)
            val shape = RoundedCornerShape(12.dp)
            Row(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(start = 20.dp, top = 12.dp, end = 8.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(pair.code, style = HqType.H2.copy(fontFamily = FontFamily.Monospace, fontSize = 34.sp, lineHeight = 44.sp, letterSpacing = 0.06.em), color = if (expired) Mts.InkMuted else Mts.Ink, modifier = Modifier.weight(1f))
                IconAction(HqIcons.Copy, "Copy the code", iconSize = 22.dp) { actions.onCopy(pair.code) }
            }
            if (!expired) {
                if (pair.deepLink != null) GreenButton("Open in ${SettingsWords.appName(platform)}") { runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(pair.deepLink)).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)) } }
                Text(SettingsWords.pairHelp(platform, pair.command, pair.deepLink != null), style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.Center)
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                    Spinner(18)
                    Text(SettingsWords.pairWaiting(SettingsWords.clock(left)), style = HqType.BodySm, color = Mts.InkMuted)
                }
            } else {
                Text(SettingsWords.PAIR_EXPIRED, style = HqType.BodySm, color = Mts.InkMuted)
                GreenButton("Get a new code") { actions.onPair(platform, false) }
            }
            ui.setup?.appId?.takeIf { platform == "discord" }?.let { id ->
                Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button) { runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(SettingsWords.inviteUrl(id))).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)) } }.padding(horizontal = 24.dp), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.settings_open_the_invite_link), style = HqType.BodySmBold, color = Mts.Ink)
                }
                if (ui.setup?.intentOff == true) Text(SettingsWords.INTENT_OFF, style = HqType.BodySm, color = Mts.Amber, textAlign = TextAlign.Center)
            }
            TextLink("Cancel", Mts.BrandBlue, onClick = actions.onBack)
        }
    }
}

@Composable
private fun KeyValue(key: String, value: String, color: Color = Mts.Ink) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(key, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.width(KEY_COLUMN))
        Text(value, style = HqType.BodySm, color = color)
    }
}

/** A word that acts, in a colour, with a 48 dp target. */
@Composable
private fun TextLink(text: String, color: Color, enabled: Boolean = true, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
        Text(text, style = HqType.BodySmBold, color = if (enabled) color else Mts.InkMuted)
    }
}

@Composable
private fun Devices(ui: SettingsUi, actions: SettingsActions) {
    val code = ui.deviceCode
    val left by produceState(0L, code) {
        while (code != null) { value = (code.expiresAt - Instant.now().epochSecond).coerceAtLeast(0); delay(1_000) }
    }
    Page {
        // Pairing does not need the web app to be up, so its row comes before the check for it.
        OpenScreen("Pair a device with a QR code", example.hq.spike.ui.pairing.PairingActivity::class.java)
        val d = ui.snap.devices
        if (Unanswered(ui, "devices", d != null) || d == null) return@Page
        val shape = RoundedCornerShape(12.dp)
        val live = code != null && left > 0
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Text(stringResource(R.string.settings_sign_in_another_device), style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.fillMaxWidth())
            if (live && code != null) {
                Text(SettingsWords.groupCode(code.code), style = HqType.H2.copy(fontFamily = FontFamily.Monospace, fontSize = 28.sp, lineHeight = 36.sp, letterSpacing = 0.06.em), color = Mts.Ink)
                Text(SettingsWords.codeHelp(SettingsWords.clock(left)), style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.Center)
                TextLink("New code", Mts.BrandBlue, enabled = ui.busy == null) { actions.onNewCode() }
            } else {
                Text(if (code != null) "That code expired." else "Get a code, then enter it on the other device.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.fillMaxWidth())
                Row(Modifier.align(Alignment.Start).heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).clickable(enabled = ui.busy == null, role = Role.Button) { actions.onNewCode() }.padding(horizontal = 24.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    if (ui.busy == "code") Spinner(18)
                    Text(stringResource(R.string.settings_new_code), style = HqType.BodySmBold, color = Mts.Ink)
                }
            }
        }
        val list = SettingsWords.devices(d)
        Label("Signed in")
        if (list.isEmpty()) Text(stringResource(R.string.settings_no_other_device_is), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 4.dp, end = 16.dp))
        else {
            Group {
                list.forEachIndexed { i, dev ->
                    if (i > 0) Divider()
                    Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 16.dp, top = 8.dp, end = 4.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(dev.label + if (dev.current) " (this phone)" else "", style = HqType.BodyBold, color = Mts.Ink)
                            Text(dev.line, style = HqType.BodySm, color = Mts.InkMuted)
                        }
                        if (!dev.current) TextLink("Sign out", Mts.Ink, enabled = ui.busy == null) { actions.onAskSignOut(dev) }
                    }
                }
            }
        }
        Box(Modifier.padding(start = 4.dp, top = 4.dp)) { TextLink("Sign out everywhere", Mts.Ink, enabled = ui.busy == null && !ui.restarting) { actions.onSignOutAll(true) } }
    }
}

// ---------------------------------------------------------------- spending limits

@Composable
private fun Spending(ui: SettingsUi) {
    Page {
        val b = ui.snap.budgets
        if (Unanswered(ui, "budgets", b != null) || b == null) return@Page
        val cards = SettingsWords.budgetCards(b, ui.now)
        Label("Budgets")
        if (cards.isEmpty()) Notice(SettingsWords.noBudgets(b), HqIcons.Info)
        else Group { cards.forEachIndexed { i, c -> if (i > 0) Divider(); BudgetCardView(c) } }
        val forecast = SettingsWords.forecast(ui.snap.forecast)
        when {
            forecast == null && "forecast" in ui.snap.unavailable -> { Label("Forecast, this month"); Text(stringResource(R.string.settings_the_forecast_is_not), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp)) }
            forecast?.note != null -> { Label("Forecast, this month"); Text(forecast.note, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp)) }
            forecast != null -> { Label("Forecast, this month"); Facts(forecast.rows) }
        }
        val services = SettingsWords.serviceRows(ui.snap)
        if (services.isNotEmpty()) {
            Label("By AI service")
            Facts(services)
        }
        val openWeb = LocalOpenWeb.current
        Text(stringResource(R.string.settings_changing_a_limit_is), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
        TextLink("Open the full usage page", Mts.BrandBlue) { openWeb(WebPage.Usage) }
    }
}

@Composable
private fun BudgetCardView(c: SettingsWords.BudgetCard) {
    val fill = when (c.tone) { SettingsWords.BudgetTone.Ok -> Mts.BrandGreen; SettingsWords.BudgetTone.Warning -> Mts.Amber; SettingsWords.BudgetTone.Exceeded -> Mts.DangerText }
    Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Bottom) {
            Text(c.title, style = HqType.BodyBold, color = Mts.Ink)
            Text(c.covers, style = SmallText, color = Mts.InkMuted, modifier = Modifier.weight(1f))
        }
        Text(c.used, style = HqType.BodySm, color = Mts.Ink)
        Box(
            Modifier.fillMaxWidth().height(BAR_HEIGHT).clip(RoundedCornerShape(BAR_HEIGHT / 2)).background(Mts.Surface200)
                .semantics { progressBarRangeInfo = ProgressBarRangeInfo(c.fraction, 0f..1f) },
        ) { Box(Modifier.fillMaxWidth(c.fraction).fillMaxHeight().background(fill)) }
        Text(c.stand, style = SmallText, color = Mts.InkMuted)
    }
}

// ---------------------------------------------------------------- background work

@Composable
private fun Background(ui: SettingsUi, actions: SettingsActions) {
    val active = ui.mode == "active"
    Page {
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ModeCard("Working in the background", "HQ answers your chats and runs scheduled tasks, even when the app is closed.", active, !ui.restarting) { actions.onMode(true) }
            ModeCard("Only when I open the app", "Your notes and tasks work. Chats and scheduled tasks pause.", !active, !ui.restarting) { actions.onMode(false) }
        }
        Box(Modifier.padding(top = 12.dp)) { Notice("Use this on one device only. If two copies of HQ share your data, both answer the same messages and run the same tasks.", HqIcons.Warning, warn = true) }
        Label("Right now")
        Facts(SettingsWords.rightNow(ui.snap, active, ui.restarting))
    }
}

/** One of the two run modes as a radio card: the whole card is the tap target, and the selected one has a green border. */
@Composable
private fun ModeCard(title: String, text: String, selected: Boolean, enabled: Boolean, onClick: () -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.fillMaxWidth().alpha(if (enabled) 1f else DISABLED_ALPHA).clip(shape).background(Mts.Surface100).border(1.dp, if (selected) Mts.BrandGreen else Mts.Line, shape)
            .selectable(selected = selected, enabled = enabled, role = Role.RadioButton, onClick = onClick).padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top,
    ) {
        Box(Modifier.padding(top = 1.dp).size(22.dp).border(2.dp, if (selected) Mts.BrandGreen else Mts.Line, CircleShape), contentAlignment = Alignment.Center) {
            if (selected) Box(Modifier.size(13.dp).background(Mts.BrandGreen, CircleShape))
        }
        Column {
            Text(title, style = HqType.BodyBold, color = Mts.Ink)
            Text(text, style = HqType.BodySm, color = Mts.InkMuted)
        }
    }
}

@Composable
private fun DialogButton(label: String, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 24.dp), contentAlignment = Alignment.Center) {
        Text(label, style = HqType.BodySmBold, color = Mts.Ink)
    }
}

// ---------------------------------------------------------------- keep running and notifications

/** A card with a bold line over a plain one, amber-bordered when something needs a look. */
@Composable
private fun NoticeCard(title: String, text: String, warn: Boolean) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, if (warn) Mts.Amber else Mts.Hairline, shape).padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top,
    ) {
        Icon(if (warn) HqIcons.Warning else HqIcons.Check, null, Modifier.padding(top = 1.dp).size(24.dp), tint = if (warn) Mts.Amber else Mts.BrandGreen)
        Column {
            Text(title, style = HqType.BodyBold, color = Mts.Ink)
            Text(text, style = HqType.BodySm, color = Mts.Ink)
        }
    }
}

@Composable
private fun KeepRunning(ui: SettingsUi, actions: SettingsActions) {
    val page = PhoneWords.keepRunning(ui.keep, ui.now)
    Page {
        Box(Modifier.padding(top = 4.dp)) { NoticeCard(page.noticeTitle, page.notice, page.warn) }
        Label("Checked")
        Group {
            page.checked.forEachIndexed { i, r ->
                if (i > 0) Divider()
                val tap = r.fix?.let { fix -> Modifier.clickable(role = Role.Button) { actions.onFix(fix) } } ?: Modifier
                Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).then(tap).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(r.title, style = HqType.BodyBold, color = Mts.Ink)
                        Text(r.value, style = HqType.BodySm, color = if (r.ok) Mts.InkMuted else Mts.Amber)
                    }
                    if (r.ok) Icon(HqIcons.Check, null, Modifier.size(22.dp), tint = Mts.BrandGreen) else Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
                }
            }
        }
        if (page.sleepingApps) {
            Label("Cannot be checked")
            Group {
                Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable(role = Role.Button) { actions.onFix(PhoneFix.SleepingApps) }.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(HqIcons.Info, null, Modifier.size(24.dp), tint = Mts.InkMuted)
                    Column(Modifier.weight(1f)) {
                        Text(stringResource(R.string.settings_sleeping_apps), style = HqType.BodyBold, color = Mts.Ink)
                        Text(stringResource(R.string.settings_samsung_may_put_agenthq), style = HqType.BodySm, color = Mts.InkMuted)
                    }
                    Icon(HqIcons.External, null, Modifier.size(20.dp), tint = Mts.InkMuted)
                }
            }
        }
        Label("History")
        Facts(listOf(page.history))
    }
}

@Composable
internal fun Notifications(ui: SettingsUi, actions: SettingsActions) {
    val n = ui.notif
    var asking by remember { androidx.compose.runtime.mutableStateOf(false) }
    // The one-time card is shown on the first visit; the stored note goes once it has been on screen.
    LaunchedEffect(n.upgraderNote) { if (n.upgraderNote) actions.onUpgraderNoteSeen() }
    Page {
        if (n.upgraderNote) {
            Box(Modifier.padding(top = 4.dp)) { InfoCard(stringResource(R.string.notif_upgrader_title), stringResource(R.string.notif_upgrader_body)) }
        }
        val notif = LocalNotif.current
        val turnOn: () -> Unit = { notif?.turnOn(NotifOrigin.Later) ?: if (n.allowed) actions.onChannel(AlertPoster.NEEDS) else actions.onFix(PhoneFix.Notifications) }
        if (!n.allowed) {
            Box(Modifier.padding(top = 4.dp)) { NoticeCard(stringResource(R.string.notify_app_off_title), stringResource(R.string.notify_app_off_body), true) }
            Box(Modifier.padding(start = 16.dp, top = 8.dp)) { DialogButton(stringResource(R.string.notify_turn_on), onClick = turnOn) }
        } else if (n.needsImportance == 0) {
            Box(Modifier.padding(top = 4.dp)) { NoticeCard(stringResource(R.string.notify_needs_off_title), stringResource(R.string.notify_needs_off_body), true) }
            Box(Modifier.padding(start = 16.dp, top = 8.dp)) { DialogButton(stringResource(R.string.notify_turn_on), onClick = turnOn) }
        } else if (n.needsImportance in 1..3) {
            Text(
                stringResource(if (n.needsImportance == 3) R.string.notify_needs_default_note else R.string.notify_needs_quiet_note),
                style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 4.dp, end = 20.dp),
            )
        }
        Label(stringResource(R.string.notif_group_show))
        DetailsRow(n, onChange = { on -> if (on) asking = true else actions.onDetails(false) })
        if (n.secure) {
            val compact = LocalConfiguration.current.let { it.fontScale >= 2f && it.screenWidthDp <= 360 }
            Text(stringResource(if (compact) R.string.notif_details_note_short else R.string.notif_details_note), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
        } else {
            Text(stringResource(R.string.notif_no_lock_note), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
            Box(Modifier.padding(start = 8.dp, top = 4.dp)) { DialogButton(stringResource(R.string.notif_open_android_settings)) { actions.onFix(PhoneFix.ScreenLock) } }
            Text(stringResource(R.string.notif_then_screen_lock), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, end = 20.dp))
        }
        Label(stringResource(R.string.notif_group_channels))
        Group {
            n.channels.forEachIndexed { i, c ->
                if (i > 0) Divider()
                Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable(role = Role.Button) { actions.onChannel(c.id) }.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(stringResource(PhoneWords.channelNameRes(c.id)), style = HqType.BodyBold, color = Mts.Ink)
                        Text(stringResource(PhoneWords.channelLineRes(c, appOff = !n.allowed)), style = HqType.BodySm, color = Mts.InkMuted)
                    }
                    Icon(HqIcons.External, null, Modifier.size(20.dp), tint = Mts.InkMuted)
                }
            }
        }
        ServerHqSection(n.server, ui, actions)
    }
    if (asking) DetailsDialog(onKeep = { asking = false }, onShow = { asking = false; actions.onDetails(true) })
}

/** The details switch. With no secure screen lock it is dimmed, drawn off and cannot be turned on. */
@Composable
private fun DetailsRow(n: NotifFacts, onChange: (Boolean) -> Unit) {
    val config = LocalConfiguration.current
    val stacked = config.fontScale >= 2f && config.screenWidthDp <= 360
    val on = n.showDetails && n.secure
    val line = stringResource(if (!n.secure) R.string.notif_no_lock_line else if (on) R.string.notif_details_on_line else R.string.notif_details_off_line)
    val toggle: @Composable () -> Unit = {
        Switch(
            checked = on, onCheckedChange = onChange, enabled = n.secure,
            colors = SwitchDefaults.colors(
                checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen, checkedBorderColor = Mts.BrandGreen,
                uncheckedTrackColor = Mts.Surface200, uncheckedThumbColor = Mts.Line, uncheckedBorderColor = Mts.Line,
            ),
        )
    }
    val words: @Composable (Modifier) -> Unit = { m ->
        Column(m) {
            Text(stringResource(R.string.notif_details_title), style = HqType.BodyBold, color = Mts.Ink)
            Text(line, style = HqType.BodySm, color = Mts.InkMuted)
        }
    }
    Group {
        Box(Modifier.alpha(if (n.secure) 1f else 0.5f)) {
            if (stacked) {
                Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { toggle(); words(Modifier) }
            } else {
                Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    words(Modifier.weight(1f)); toggle()
                }
            }
        }
    }
}

/** Asked once when details are turned on. Keep hidden, Back and a tap outside leave them off. */
@Composable
private fun DetailsDialog(onKeep: () -> Unit, onShow: () -> Unit) {
    androidx.compose.ui.window.Dialog(onDismissRequest = onKeep) {
        Column(Modifier.clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.notif_dialog_title), style = HqType.H2, color = Mts.Ink)
            Text(stringResource(R.string.notif_dialog_body_unlocked), style = HqType.BodySm, color = Mts.InkMuted)
            Text(stringResource(R.string.notif_dialog_body_allow), style = HqType.BodySm, color = Mts.InkMuted)
            Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedChoice(stringResource(R.string.notif_keep_hidden), Modifier.weight(1f), onKeep)
                OutlinedChoice(stringResource(R.string.notif_show_details), Modifier.weight(1f), onShow)
            }
        }
    }
}

@Composable
private fun OutlinedChoice(label: String, modifier: Modifier, onClick: () -> Unit) {
    Box(modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp), contentAlignment = Alignment.Center) {
        Text(label, style = HqType.BodySmBold, color = Mts.Ink)
    }
}

/** A quiet card with an info icon, for a note that is not a warning. */
@Composable
private fun InfoCard(title: String, text: String) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top,
    ) {
        Icon(HqIcons.Info, null, Modifier.padding(top = 1.dp).size(24.dp), tint = Mts.InkMuted)
        Column {
            Text(title, style = HqType.BodyBold, color = Mts.Ink)
            Text(text, style = HqType.BodySm, color = Mts.Ink)
        }
    }
}

/** The owner's HQ on a server: this phone signs in to it with a one-time code, and its questions then arrive here too. */
@Composable
private fun ServerHqSection(server: ServerFacts, ui: SettingsUi, actions: SettingsActions) {
    Label("Your HQ on a server")
    if (server.connected) {
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_signed_in), style = HqType.BodyBold, color = Mts.Ink)
                    Text(server.address.orEmpty(), style = HqType.BodySm, color = Mts.InkMuted)
                }
                DialogButton("Disconnect") { actions.onAskServerDisconnect(true) }
            }
        }
        Text(
            stringResource(R.string.settings_questions_from_coding_agents),
            style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp),
        )
        return
    }
    if (server.signedOut) Box(Modifier.padding(top = 4.dp)) { NoticeCard("Signed out", "Your HQ at ${server.address} signed this phone out. Sign in again with a new code.", true) }
    val busy = ui.busy == SERVER_BUSY
    var address by remember { androidx.compose.runtime.mutableStateOf(server.address.orEmpty()) }
    var code by remember { androidx.compose.runtime.mutableStateOf("") }
    Column(Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.settings_if_your_hq_runs), style = HqType.BodySm, color = Mts.InkMuted)
        ServerField("Address", address, "https://hq.example.com", busy, androidx.compose.ui.text.input.KeyboardType.Uri) { address = it }
        ServerField("One-time code", code, "XXXXX-XXXXX", busy, androidx.compose.ui.text.input.KeyboardType.Ascii) { code = it }
        ui.serverError?.let { Text(it, style = HqType.BodySm, color = Mts.DangerText) }
        GreenButton(if (busy) "Signing in\u2026" else "Sign in", enabled = !busy && address.isNotBlank() && code.isNotBlank(), busy = busy) { actions.onServerConnect(address, code) }
    }
}

/** One labelled line of text for the server sign-in, with a hint while it is empty. */
@Composable
private fun ServerField(label: String, value: String, hint: String, busy: Boolean, keyboard: androidx.compose.ui.text.input.KeyboardType, onChange: (String) -> Unit) {
    val field = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(field).background(Mts.Surface000).border(2.dp, Mts.Line, field).padding(horizontal = 16.dp, vertical = 8.dp)) {
        Text(label, style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
        Box {
            if (value.isEmpty()) Text(hint, style = HqType.Body, color = Mts.InkMuted)
            androidx.compose.foundation.text.BasicTextField(
                value, onChange, Modifier.fillMaxWidth(), enabled = !busy, singleLine = true,
                textStyle = HqType.Body.copy(color = Mts.Ink),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(Mts.Ink),
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = keyboard, autoCorrectEnabled = false),
            )
        }
    }
}

// ---------------------------------------------------------------- add a key

/** The one place a key is typed: a sheet over Settings. The key lives only in this field, is shown as dots unless revealed, and goes away with the sheet. */
@Composable
private fun KeySheetView(sheet: KeySheet, actions: SettingsActions) {
    var typed by remember { androidx.compose.runtime.mutableStateOf("") }
    var reveal by remember { androidx.compose.runtime.mutableStateOf(false) }
    val words = SettingsWords.keyWords(sheet.name, sheet.replace)
    val context = androidx.compose.ui.platform.LocalContext.current
    val shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
    val field = RoundedCornerShape(12.dp)
    val bad = sheet.error != null
    BackHandler(enabled = !sheet.checking, onBack = actions.onKeyClose)
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = SCRIM_ALPHA)).clickable(enabled = !sheet.checking, interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = actions.onKeyClose))
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().clip(shape).background(Mts.Surface100).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {}
                .imePadding().navigationBarsPadding().padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(Modifier.align(Alignment.CenterHorizontally).size(32.dp, 4.dp).clip(RoundedCornerShape(2.dp)).background(Mts.Line))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(words.title, style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                IconAction(HqIcons.Close, "Close", iconSize = 24.dp, onClick = actions.onKeyClose)
            }
            Text(words.body, style = HqType.BodySm, color = Mts.InkMuted)
            Row(
                Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(field).background(Mts.Surface000).border(2.dp, if (bad) Mts.DangerText else Mts.Line, field).padding(start = 16.dp, top = 8.dp, end = 8.dp, bottom = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_your_key), style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = if (bad) Mts.DangerText else Mts.InkMuted)
                    androidx.compose.foundation.text.BasicTextField(
                        typed, { typed = it }, Modifier.fillMaxWidth(), enabled = !sheet.checking, singleLine = true,
                        textStyle = HqType.Body.copy(color = Mts.Ink, fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace, letterSpacing = 0.1.em),
                        cursorBrush = androidx.compose.ui.graphics.SolidColor(Mts.Ink),
                        visualTransformation = if (reveal) androidx.compose.ui.text.input.VisualTransformation.None else androidx.compose.ui.text.input.PasswordVisualTransformation(),
                        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.Password, autoCorrectEnabled = false),
                    )
                }
                IconAction(if (reveal) HqIcons.EyeOff else HqIcons.Eye, if (reveal) "Hide the key" else "Show the key", iconSize = 22.dp) { reveal = !reveal }
            }
            sheet.error?.let { Text(it, style = HqType.BodySm, color = Mts.DangerText, modifier = Modifier.padding(start = 16.dp, top = 0.dp)) }
            SettingsWords.keyHelpUrl(sheet.id)?.let { url ->
                Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button) { runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url)).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)) } }.padding(horizontal = 12.dp), contentAlignment = Alignment.CenterStart) {
                    Text(stringResource(R.string.settings_where_do_i_find), style = HqType.BodySmBold, color = Mts.BrandBlue)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                val ready = typed.isNotBlank() && !sheet.checking
                Row(
                    Modifier.weight(1f).heightIn(min = 48.dp).clip(CircleShape).background(if (ready || sheet.checking) Mts.BrandGreen else Mts.Surface200).clickable(enabled = ready, role = Role.Button) { actions.onKeySave(typed) },
                    horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (sheet.checking) Spinner(18, Mts.OnGreen)
                    Text(if (sheet.checking) "Checking\u2026" else "Check and save", style = HqType.BodySmBold, color = if (ready || sheet.checking) Mts.OnGreen else Mts.InkMuted)
                }
                Box(Modifier.heightIn(min = 48.dp).clickable(enabled = !sheet.checking, role = Role.Button, onClick = actions.onKeyClose).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.pairing_cancel), style = HqType.BodySmBold, color = Mts.InkMuted)
                }
            }
            Text(SettingsWords.KEY_NOTE, style = SmallText, color = Mts.InkMuted)
        }
    }
}

// ---------------------------------------------------------------- capture and widgets

@Composable
private fun CaptureWidgets(ui: SettingsUi, actions: SettingsActions) {
    val c = ui.capture
    Page {
        Label("Share sheet opens on")
        Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(Triple("note", "Note", "Notebooks/Inbox"), Triple("task", "Task", "Personal \u00b7 Inbox"), Triple("last", "The last one I used", "")).forEach { (key, title, end) ->
                val on = c.shareOpens == key
                val shape = RoundedCornerShape(12.dp)
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 50.dp).clip(shape).background(Mts.Surface100).border(1.dp, if (on) Mts.BrandGreen else Mts.Line, shape).selectable(selected = on, role = Role.RadioButton) { actions.onShareOpens(key) }.padding(horizontal = 16.dp, vertical = 12.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(Modifier.size(22.dp).border(2.dp, if (on) Mts.BrandGreen else Mts.Line, CircleShape), contentAlignment = Alignment.Center) { if (on) Box(Modifier.size(13.dp).background(Mts.BrandGreen, CircleShape)) }
                    Text(title, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
                    if (end.isNotEmpty()) Text(end, style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
        }
        Label("Widgets")
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_show_titles_in_needs), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.settings_off_the_widget_shows), style = HqType.BodySm, color = Mts.InkMuted)
                }
                Switch(
                    checked = c.showTitles, onCheckedChange = actions.onShowTitles,
                    colors = SwitchDefaults.colors(checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen, checkedBorderColor = Mts.BrandGreen, uncheckedTrackColor = Mts.Surface200, uncheckedThumbColor = Mts.Line, uncheckedBorderColor = Mts.Line),
                )
            }
        }
        Group {
            listOf("Capture", "Needs you", "Tasks").forEachIndexed { i, name ->
                if (i > 0) Divider()
                Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(name, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
                    Box(Modifier.heightIn(min = 40.dp).clip(CircleShape).background(Mts.Surface200).clickable(role = Role.Button) { actions.onAddWidget(i) }.padding(horizontal = 16.dp), contentAlignment = Alignment.Center) { Text(stringResource(R.string.settings_add), style = HqType.BodySmBold, color = Mts.Ink) }
                }
            }
        }
        if (c.waiting.any { it.id !in ui.hiddenCaptures }) {
            Label("Waiting to send")
            Group {
                c.waiting.filter { it.id !in ui.hiddenCaptures }.forEachIndexed { i, q ->
                    if (i > 0) Divider()
                    Column(Modifier.fillMaxWidth().padding(start = 16.dp, top = 12.dp, end = 8.dp, bottom = 4.dp)) {
                        Text(q.title.ifBlank { "Untitled" }, style = HqType.BodyBold, color = Mts.Ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        Text(PhoneWords.queuedLine(q), style = HqType.BodySm, color = if (q.failure != null) Mts.Amber else Mts.InkMuted)
                        Row(horizontalArrangement = Arrangement.spacedBy(0.dp)) {
                            DialogButton("Send now") { actions.onSendNow(q.id) }
                            DialogButton("Discard") { actions.onDiscard(q.id) }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- about

@Composable
private fun About(ui: SettingsUi, actions: SettingsActions) {
    Page {
        BackupCard(BackupWords.card(ui.backup, Instant.now()))
        MovingCard()
        Label("About")
        Facts(
            listOf(
                FactRow("AgentHQ", ui.build),
                FactRow("Run mode", if (ui.mode == "active") "Active" else "Standby"),
                FactRow("HQ", ui.snap.health?.optString("version")?.takeIf { it.isNotBlank() } ?: "Not answering"),
            ) + listOfNotNull(SettingsWords.notesCount(ui.snap)?.let { FactRow("Notes", it) }),
        )
        Label("Updates")
        UpdateGroup(ui, actions)
        Label("Support")
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { actions.onPage(SettingsPage.Technical) }.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_technical_title), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.settings_technical_sub), style = HqType.BodySm, color = Mts.InkMuted)
                }
                Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
            Divider()
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(enabled = !ui.restarting, role = Role.Button) { actions.onRestart(true) }.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(if (ui.restarting) "Restarting HQ\u2026" else "Restart HQ", style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
                if (ui.restarting) Spinner(20) else Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
        }
    }
}

// ---------------------------------------------------------------- technical details and what HQ noticed

private val NARROW_DP = 360
private val CELL_WIDTH = 72.dp

@Composable
private fun Technical(ui: SettingsUi, actions: SettingsActions) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val counters = remember { Counters.get(context) }
    val info = remember { context.packageManager.getPackageInfo(context.packageName, 0) }
    val code = if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.P) info.longVersionCode else @Suppress("DEPRECATION") info.versionCode.toLong()
    val version = info.versionName.orEmpty().ifBlank { "dev" }
    val hqVersion = ui.snap.health?.optString("version")?.takeIf { it.isNotBlank() }
    val hq = if (ui.snap.health != null) stringResource(R.string.state_connected) else stringResource(R.string.technical_hq_not_answering)
    var sheet by remember { mutableStateOf<String?>(null) }
    val facts = SettingsWords.DetailFacts(version, code.toString(), android.os.Build.VERSION.RELEASE.orEmpty(), ui.mode == "active", hq)
    Page {
        Facts(
            listOf(
                FactRow(stringResource(R.string.technical_version), hqVersion ?: stringResource(R.string.technical_hq_not_answering)),
                FactRow(stringResource(R.string.technical_build), code.toString()),
                FactRow(stringResource(R.string.technical_android), facts.android),
                FactRow(stringResource(R.string.technical_hq), hq),
            ),
        )
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { actions.onPage(SettingsPage.Noticed) }.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.noticed_title), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.noticed_sub), style = HqType.BodySm, color = Mts.InkMuted)
                }
                Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
            Divider()
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { sheet = SettingsWords.copyDetails(facts, counters.snapshot()) }.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(HqIcons.Copy, null, Modifier.size(22.dp), tint = Mts.Ink)
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.copy_details_title), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.copy_details_sub), style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
        }
        Text(stringResource(R.string.technical_note), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp))
    }
    sheet?.let { CopyDetailsSheet(it) { sheet = null } }
}

/** Shows the exact text before it goes on the clipboard; Copy puts that text there and nothing else. */
@Composable
internal fun CopyDetailsSheet(text: String, onClose: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val copied = stringResource(R.string.copy_details_copied)
    BackHandler(onBack = onClose)
    Sheet(onClose) {
        Text(stringResource(R.string.copy_details_title), style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(bottom = 8.dp))
        Text(stringResource(R.string.copy_details_body), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(bottom = 12.dp))
        Box(Modifier.weight(1f, fill = false).fillMaxWidth().clip(RoundedCornerShape(8.dp)).background(Mts.Surface100).verticalScroll(rememberScrollState()).padding(12.dp)) {
            Text(text, style = SmallText, fontFamily = FontFamily.Monospace, color = Mts.Ink)
        }
        Row(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
            DialogButton(stringResource(R.string.copy_details_not_now), onClose)
            DialogButton(stringResource(R.string.copy_details_copy)) {
                context.getSystemService(android.content.ClipboardManager::class.java).setPrimaryClip(android.content.ClipData.newPlainText("HQ details", text))
                android.widget.Toast.makeText(context, copied, android.widget.Toast.LENGTH_SHORT).show()
                onClose()
            }
        }
    }
}

/** A card at the foot of the page over a scrim; a tap outside closes it. */
@Composable
private fun Sheet(onClose: () -> Unit, content: @Composable androidx.compose.foundation.layout.ColumnScope.() -> Unit) {
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = SCRIM_ALPHA)).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClose))
        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().heightIn(max = 560.dp).clip(RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)).background(Mts.Surface200).navigationBarsPadding().padding(24.dp), content = content)
    }
}

/** The counters, this version and the last, read only from the counter file: it works with HQ stopped. */
@Composable
internal fun NoticedPage(counters: Counters) {
    val context = androidx.compose.ui.platform.LocalContext.current
    var snap by remember { mutableStateOf(counters.snapshot()) }
    var line by remember { mutableStateOf<NoticeLine?>(null) }
    var askClear by remember { mutableStateOf(false) }
    val config = LocalConfiguration.current
    val narrow = config.screenWidthDp <= NARROW_DP && config.fontScale >= 2f
    val words = CellWords({ a, b -> context.getString(R.string.noticed_of, a.toString(), b.toString()) }, stringResource(R.string.noticed_yes), stringResource(R.string.noticed_no))
    val asksPermission = android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU
    val notAsked = stringResource(R.string.noticed_not_asked)
    Page {
        Text(stringResource(R.string.noticed_intro), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp))
        if (!snap.on) Box(Modifier.padding(top = 12.dp)) { Notice(stringResource(R.string.noticed_off_note, Noticed.day(snap.stoppedOn)), HqIcons.Info, warn = true) }
        Row(Modifier.fillMaxWidth().padding(start = 32.dp, top = 16.dp, end = 32.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.noticed_since, Noticed.day(snap.current.since)), style = HqType.BodySmBold, color = Mts.InkMuted, modifier = Modifier.weight(1f))
            Text(snap.current.version, style = HqType.BodySmBold, color = Mts.Ink, textAlign = TextAlign.End, modifier = Modifier.width(CELL_WIDTH))
            if (!narrow) snap.previous?.let { Text(it.version, style = HqType.BodySmBold, color = Mts.InkMuted, textAlign = TextAlign.End, modifier = Modifier.width(CELL_WIDTH)) }
        }
        if (narrow) snap.previous?.let { Text(stringResource(R.string.noticed_narrow_caption, it.version), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 32.dp, end = 32.dp)) }
        Noticed.lines.groupBy { it.group }.forEach { (group, rows) ->
            Label(stringResource(group))
            Group {
                rows.forEachIndexed { i, l ->
                    if (i > 0) Divider()
                    val skipped = l.asksPermission && !asksPermission
                    val now = if (skipped) notAsked else Noticed.cell(snap.current, l, words)
                    val before = if (skipped) null else snap.previous?.let { Noticed.cell(it, l, words) }
                    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button) { line = l }.padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(stringResource(l.label), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.weight(1f))
                        if (narrow) {
                            Column(horizontalAlignment = Alignment.End) {
                                Text(now, style = HqType.BodySmBold, color = Mts.Ink, textAlign = TextAlign.End)
                                before?.let { Text(it, style = SmallText, color = Mts.InkMuted, textAlign = TextAlign.End) }
                            }
                        } else {
                            Text(now, style = HqType.BodySmBold, color = Mts.Ink, textAlign = TextAlign.End, modifier = if (skipped) Modifier.widthIn(max = VALUE_MAX) else Modifier.width(CELL_WIDTH))
                            if (!skipped) before?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.End, modifier = Modifier.width(CELL_WIDTH)) }
                        }
                    }
                }
            }
        }
        Label(stringResource(R.string.noticed_switch))
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Switch) { counters.setOn(!snap.on); snap = counters.snapshot() }.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.noticed_switch), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(if (snap.on) R.string.noticed_switch_on else R.string.noticed_switch_off), style = HqType.BodySm, color = Mts.InkMuted)
                }
                Switch(
                    checked = snap.on, onCheckedChange = null,
                    colors = SwitchDefaults.colors(checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen, checkedBorderColor = Mts.BrandGreen, uncheckedTrackColor = Mts.Surface200, uncheckedThumbColor = Mts.Line, uncheckedBorderColor = Mts.Line),
                )
            }
            Divider()
            Column(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { askClear = true }.padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.Center) {
                Text(stringResource(R.string.noticed_clear), style = HqType.BodyBold, color = Mts.Ink)
                Text(stringResource(R.string.noticed_clear_sub), style = HqType.BodySm, color = Mts.InkMuted)
            }
        }
        Text(stringResource(R.string.noticed_foot), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp))
    }
    line?.let { l ->
        BackHandler { line = null }
        Sheet({ line = null }) {
            Text(stringResource(l.label), style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(bottom = 8.dp))
            Text(stringResource(l.what), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(bottom = 8.dp))
            Text(stringResource(l.only), style = HqType.BodySm, color = Mts.InkMuted)
            Row(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalArrangement = Arrangement.End) { DialogButton(stringResource(R.string.noticed_done)) { line = null } }
        }
    }
    if (askClear) {
        BackHandler { askClear = false }
        Box(Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = SCRIM_ALPHA)).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) { askClear = false })
            Column(Modifier.align(Alignment.Center).padding(24.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp)) {
                Text(stringResource(R.string.noticed_clear_title), style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(bottom = 12.dp))
                Text(stringResource(R.string.noticed_clear_body), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(bottom = 20.dp))
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                    DialogButton(stringResource(R.string.noticed_clear_keep)) { askClear = false }
                    DialogButton(stringResource(R.string.noticed_clear_confirm)) { counters.clear(); snap = counters.snapshot(); askClear = false }
                }
            }
        }
    }
}

/** Checks for a newer AgentHQ build on GitHub Releases; the install only starts when the owner taps it. */
@Composable
private fun UpdateGroup(ui: SettingsUi, actions: SettingsActions) {
    val state = ui.update
    val working = state == UpdateState.Checking || state == UpdateState.Downloading
    Group {
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(enabled = !working, role = Role.Button) { if (state is UpdateState.Available) actions.onInstallUpdate(state.manifest) else actions.onCheckUpdate() }.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                Text(if (state is UpdateState.Available) "Install update" else "Check for updates", style = HqType.BodyBold, color = Mts.Ink)
                Text(UpdateLogic.wordsFor(androidx.compose.ui.platform.LocalContext.current, state), style = HqType.BodySm, color = if (state is UpdateState.Failed) Mts.Amber else Mts.InkMuted)
            }
            if (working) Spinner(20) else Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
        }
        if (state is UpdateState.Busy) {
            example.hq.spike.update.InstallBusyDialog(
                state.reason,
                onWait = { actions.onWaitThenInstall(state.manifest) },
                onStop = { actions.onStopAndInstall(state.manifest) },
                onDismiss = { actions.onBusyDismiss(state.manifest) },
            )
        }
        Divider()
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Switch) { actions.onUpdateMain(!ui.updateMain) }.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                Text(stringResource(R.string.settings_follow_every_build_of), style = HqType.BodyBold, color = Mts.Ink)
                Text(if (ui.updateMain) "Newest builds, before they are promoted" else "Only builds the owner promoted", style = HqType.BodySm, color = Mts.InkMuted)
            }
            Text(if (ui.updateMain) "On" else "Off", style = HqType.BodySmBold, color = Mts.Ink)
        }
    }
}

/** What backups on this phone are doing; the button to set one up is drawn but off, since the app cannot set it up yet. */
@Composable
private fun BackupCard(card: BackupWords.Card) {
    val shape = RoundedCornerShape(12.dp)
    val warn = card.tone == BackupWords.Tone.Warn
    val tint = if (warn) Mts.Amber else Mts.BrandGreen
    Column(
        Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, if (warn) Mts.Amber else Mts.Line, shape).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(if (warn) HqIcons.Warning else HqIcons.Check, null, Modifier.size(22.dp), tint = tint)
            Text(card.title, style = HqType.BodyBold, color = Mts.Ink)
        }
        Text(card.body, style = HqType.BodySm, color = Mts.Ink)
        if (card.offerSetUp) Row(
            Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).padding(horizontal = 24.dp).semantics { disabled(); contentDescription = "Set up backup, needs a decision" },
            horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(stringResource(R.string.settings_set_up_backup), style = HqType.BodySmBold, color = Mts.Ink)
            Box(Modifier.heightIn(min = 24.dp).clip(RoundedCornerShape(12.dp)).border(1.dp, Mts.Line, RoundedCornerShape(12.dp)).padding(horizontal = 10.dp), contentAlignment = Alignment.Center) {
                Text(stringResource(R.string.settings_needs_a_decision), style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
            }
        }
    }
}

/** What the old phone should know before a move: only notes travel, and the copy button waits for the safe-mode .zip. */
@Composable
private fun MovingCard() {
    val shape = RoundedCornerShape(12.dp)
    Column(
        Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(stringResource(R.string.old_phone_title), style = HqType.BodyBold, color = Mts.Ink)
        Text(stringResource(R.string.old_phone_body), style = HqType.BodySm, color = Mts.Ink)
        Text(stringResource(R.string.old_phone_older), style = HqType.BodySm, color = Mts.Ink)
        Row(
            Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).padding(horizontal = 24.dp).semantics { disabled() },
            verticalAlignment = Alignment.CenterVertically,
        ) { Text(stringResource(R.string.old_phone_save_copy), style = HqType.BodySmBold, color = Mts.InkMuted) }
        Text(stringResource(R.string.old_phone_note), style = HqType.BodySm, color = Mts.InkMuted)
    }
}

/** A read-only list pushed from a Safety row: the standing notice, then its rows. */
@Composable
private fun ReadOnlyList(ui: SettingsUi, empty: String, rows: @Composable () -> Unit) {
    Page {
        Notice("Read only. You can change these in the config file on this phone.")
        if (ui.snap.settings == null) Unanswered(ui, "settings", false)
        else Box(Modifier.padding(top = 12.dp)) { Column { rows() } }
    }
}

/** An address or other literal text, as it is written. */
@Composable
private fun MonoRow(text: String) {
    Group { Text(text, style = HqType.BodySm.copy(fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace), color = Mts.Ink, modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 16.dp)) }
}
