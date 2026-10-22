package example.hq.spike.ui.settings

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
import androidx.compose.foundation.layout.navigationBarsPadding
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
import androidx.compose.runtime.remember
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.consumeTouches
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.delay

private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private const val LINE_MS = 4_000L
private const val REFRESH_MS = 15_000L
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
    "plug" -> HqIcons.Plug
    "phone" -> HqIcons.Phone
    "shield" -> HqIcons.Shield
    "gauge" -> HqIcons.Gauge
    else -> HqIcons.Archive
}

private fun title(page: SettingsPage) = when (page) {
    SettingsPage.Home -> "Settings"
    SettingsPage.Safety -> "Safety and limits"
    SettingsPage.Keys -> "AI service and keys"
    SettingsPage.Messaging -> "Messaging apps"
    SettingsPage.Claude -> "Connect Claude"
    SettingsPage.Devices -> "Other devices"
    SettingsPage.Spending -> "Spending limits"
    SettingsPage.About -> "About"
    SettingsPage.Background -> "Background work"
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
                Text(title(ui.page), style = HqType.ScreenTitle, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 4.dp))
            }
            when (ui.page) {
                SettingsPage.Home -> Home(ui, actions)
                SettingsPage.Safety -> Safety(ui, actions)
                SettingsPage.Keys -> Keys(ui)
                SettingsPage.Messaging -> Messaging(ui)
                SettingsPage.Claude -> Claude(ui)
                SettingsPage.Devices -> Devices(ui)
                SettingsPage.Spending -> Spending(ui)
                SettingsPage.About -> About(ui, actions)
                SettingsPage.Background -> Background(ui, actions)
            }
        }
        ui.askMode?.let { ModeDialog(it, actions) }
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
            Text(row.status, style = HqType.BodySm, color = if (row.warn) Mts.Amber else Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (row.warn) Icon(HqIcons.Warning, null, Modifier.size(20.dp), tint = Mts.Amber)
        Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

@Composable
private fun Fact(row: FactRow, first: Boolean) {
    if (!first) Divider()
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(row.title, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
            Text(row.value, style = HqType.BodySm, color = if (row.good) Mts.BrandGreen else Mts.InkMuted, textAlign = TextAlign.End, modifier = Modifier.widthIn(max = VALUE_MAX))
        }
        row.text?.let { Text(it, style = SmallText, color = Mts.InkMuted) }
    }
}

@Composable
private fun Facts(rows: List<FactRow>) = Group { rows.forEachIndexed { i, r -> Fact(r, i == 0) } }

@Composable
private fun Waiting(ui: SettingsUi) {
    if (!ui.loaded) Text("Asking HQ…", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
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
        SettingsWords.home(ui.snap, ui.now, ui.phone, ui.loaded).forEachIndexed { gi, g ->
            Text(g.title.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = if (gi == 0) 8.dp else 20.dp, end = 16.dp, bottom = 4.dp))
            Group {
                g.rows.forEachIndexed { i, r -> NavRow(r, i == 0) { if (r.page != null) actions.onPage(r.page) else actions.onPhoneRow(r.key) } }
            }
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
            Box(Modifier.padding(top = 12.dp)) { if (rows.isEmpty()) Unanswered(ui, "settings", false) else Facts(rows) }
        }
    }
}

// ---------------------------------------------------------------- AI service and keys

@Composable
private fun Keys(ui: SettingsUi) {
    val s = ui.snap.settings
    Page {
        if (Unanswered(ui, "settings", s != null) || s == null) return@Page
        Label("Model")
        Facts(listOf(FactRow("Which AI HQ uses", SettingsWords.activeModel(s)), FactRow("Only AI that runs on this phone", SettingsWords.onOff(s.optJSONObject("model")?.optBoolean("local_only") == true))))
        Text("The model is set in the config file on this phone.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
        val ai = SettingsWords.aiRows(ui.snap)
        if (ai.isNotEmpty()) {
            Label("Which AI answers")
            Facts(ai)
            Text("HQ tries them in this order and uses the first one that answers.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
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
                    if (set) Icon(HqIcons.CheckSmall, null, Modifier.size(22.dp), tint = Mts.BrandGreen)
                }
            }
        }
        Text("Keys are added from the web app for now. They are never shown here.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
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
private fun Messaging(ui: SettingsUi) {
    Page {
        if (Unanswered(ui, "chatApps", ui.snap.chatApps != null)) return@Page
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            listOf("telegram" to "Telegram", "discord" to "Discord").forEach { (key, name) -> AppCardView(SettingsWords.appCard(ui.snap.chatApps, key, name)) }
        }
        Text("Setting up or changing a messaging app is done in the web app for now.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
    }
}

@Composable
private fun AppCardView(c: SettingsWords.AppCard) {
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
        } else Text("Add a bot so HQ can answer in ${c.name}.", style = HqType.BodySm, color = Mts.InkMuted)
    }
}

@Composable
private fun KeyValue(key: String, value: String, color: Color = Mts.Ink) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(key, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.width(KEY_COLUMN))
        Text(value, style = HqType.BodySm, color = color)
    }
}

// ---------------------------------------------------------------- Connect Claude

@Composable
private fun Claude(ui: SettingsUi) {
    val c = ui.snap.connector
    val armed = SettingsWords.time(c, "armed_until")?.isAfter(ui.now) == true
    // Between reads the clock stands still; while a window is open it is the countdown, so it is kept moving.
    val now by produceState(ui.now, armed, ui.now) {
        value = ui.now
        while (armed) { delay(1_000); value = Instant.now() }
    }
    Page {
        if (Unanswered(ui, "connector", c != null) || c == null) return@Page
        if (!c.optBoolean("enabled")) {
            Notice("Set mcp_connector.enabled and mcp_connector.public_url in the config, then restart HQ.", HqIcons.Info)
            return@Page
        }
        val shape = RoundedCornerShape(12.dp)
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("ADDRESS TO GIVE CLAUDE", style = HqType.Group.copy(fontSize = 12.sp, letterSpacing = 0.06.em), color = Mts.InkMuted)
                Text(c.optString("url", "Not set"), style = HqType.BodySm.copy(fontFamily = FontFamily.Monospace), color = Mts.Ink)
            }
            Text(SettingsWords.connectorStatus(c, now), style = HqType.BodyBold, color = Mts.Ink)
        }
        Label("Connected")
        val grants = SettingsWords.grants(c, now)
        if (grants.isEmpty()) Text("Nothing is connected.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp))
        else Group {
            grants.forEachIndexed { i, g ->
                if (i > 0) Divider()
                Column(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp)) {
                    Text(g.client, style = HqType.BodyBold, color = Mts.Ink)
                    Text("${g.scope} · connected ${g.since}", style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
        }
        Text("Opening the connection window and disconnecting are done in the web app for now.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
    }
}

// ---------------------------------------------------------------- other devices

@Composable
private fun Devices(ui: SettingsUi) {
    Page {
        val d = ui.snap.devices
        if (Unanswered(ui, "devices", d != null) || d == null) return@Page
        val list = SettingsWords.devices(d)
        if (list.isEmpty()) Text("No other device is signed in.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
        else Group {
            list.forEachIndexed { i, dev ->
                if (i > 0) Divider()
                Column(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp)) {
                    Text(dev.label + if (dev.current) " (this phone)" else "", style = HqType.BodyBold, color = Mts.Ink)
                    Text(dev.line, style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
        }
        Text("Signing in another device and signing devices out are done in the web app for now.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
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
            forecast == null && "forecast" in ui.snap.unavailable -> { Label("Forecast, this month"); Text("The forecast is not available here.", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp)) }
            forecast?.note != null -> { Label("Forecast, this month"); Text(forecast.note, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp)) }
            forecast != null -> { Label("Forecast, this month"); Facts(forecast.rows) }
        }
        val services = SettingsWords.serviceRows(ui.snap)
        if (services.isNotEmpty()) {
            Label("By AI service")
            Facts(services)
        }
        Text("Changing a limit, and the full usage page, are in the web app for now.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
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

/** Says what stops or starts before the run mode changes, over the whole of Settings. Back and a tap outside cancel. */
@Composable
private fun ModeDialog(toActive: Boolean, actions: SettingsActions) {
    val words = SettingsWords.modeDialog(toActive)
    BackHandler(onBack = actions.onModeCancel)
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = SCRIM_ALPHA)).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = actions.onModeCancel))
        Column(Modifier.padding(start = 24.dp, top = 270.dp, end = 24.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp)) {
            Text(words.title, style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(bottom = 12.dp))
            Text(words.body, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(bottom = 20.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                DialogButton("Cancel", actions.onModeCancel)
                DialogButton(words.confirm, actions.onModeConfirm)
            }
        }
    }
}

@Composable
private fun DialogButton(label: String, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 24.dp), contentAlignment = Alignment.Center) {
        Text(label, style = HqType.BodySmBold, color = Mts.Ink)
    }
}

// ---------------------------------------------------------------- about

@Composable
private fun About(ui: SettingsUi, actions: SettingsActions) {
    Page {
        Notice("Backup is set in the config file on this phone. If the app is uninstalled or the phone is lost, your notes and tasks go with it unless a backup was made.", HqIcons.Warning, warn = true)
        Label("About")
        Facts(
            listOf(
                FactRow("AgentHQ", ui.build),
                FactRow("Run mode", if (ui.mode == "active") "Active" else "Standby"),
                FactRow("HQ", ui.snap.health?.optString("version")?.takeIf { it.isNotBlank() } ?: "Not answering"),
            ),
        )
        Label("Support")
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button, onClick = actions.onCopyDiagnostics).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(HqIcons.Copy, null, Modifier.size(22.dp), tint = Mts.Ink)
                Text("Copy diagnostics", style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
            }
        }
    }
}
