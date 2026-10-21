package example.hq.spike.ui.settings

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
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
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
}

/** Settings over the whole shell, with its own pages: Back goes up one page, then closes. */
@Composable
fun SettingsHost(ui: SettingsUi, actions: SettingsActions) {
    if (!ui.open) return
    BackHandler(onBack = actions.onBack)
    Box(Modifier.fillMaxSize().background(Mts.Surface000).statusBarsPadding().navigationBarsPadding()) {
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
            }
        }
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
            Text(row.value, style = HqType.BodySm, color = Mts.InkMuted)
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

@Composable
private fun NoAnswer(text: String = "HQ is not running, so there is nothing to show yet. Try again when it is back.") =
    Notice(text, HqIcons.WifiOff, warn = true)

// ---------------------------------------------------------------- home

@Composable
private fun Home(ui: SettingsUi, actions: SettingsActions) {
    Page {
        SettingsWords.home(ui.snap, ui.now, ui.phone).forEachIndexed { gi, g ->
            Text(g.title.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = if (gi == 0) 8.dp else 20.dp, end = 16.dp, bottom = 4.dp))
            Group {
                g.rows.forEachIndexed { i, r -> NavRow(r, i == 0) { if (r.page != null) actions.onPage(r.page) else actions.onPhoneRow(r.key) } }
            }
        }
        Waiting(ui)
    }
}

// ---------------------------------------------------------------- safety and limits

@Composable
private fun Safety(ui: SettingsUi, actions: SettingsActions) {
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SettingsWords.SafetyTab.entries.forEach { t ->
                val on = t == ui.safetyTab
                Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(role = Role.Tab, onClickLabel = t.label) { actions.onTab(t) }, contentAlignment = Alignment.Center) {
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
            Box(Modifier.padding(top = 12.dp)) { if (rows.isEmpty()) NoAnswer() else Facts(rows) }
            Waiting(ui)
        }
    }
}

// ---------------------------------------------------------------- AI service and keys

@Composable
private fun Keys(ui: SettingsUi) {
    val s = ui.snap.settings
    Page {
        if (s == null) { NoAnswer(); Waiting(ui); return@Page }
        Label("Model")
        Facts(listOf(FactRow("Which AI HQ uses", SettingsWords.activeModel(s)), FactRow("Only AI that runs on this phone", SettingsWords.onOff(s.optJSONObject("model")?.optBoolean("local_only") == true))))
        Text("The model is set in the config file on this phone.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
        Label("Your keys")
        Group {
            SettingsWords.keyRows(s).forEachIndexed { i, (name, set, source) ->
                if (i > 0) Divider()
                Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(name, style = HqType.BodyBold, color = Mts.Ink)
                        Text(if (set) (if (source == "env") "Set outside the app" else "Set") else "Not set", style = HqType.BodySm, color = if (set) Mts.BrandGreen else Mts.InkMuted)
                    }
                    if (set) Icon(HqIcons.CheckSmall, null, Modifier.size(22.dp), tint = Mts.BrandGreen)
                }
            }
        }
        Text("Keys are added from the web app for now. They are never shown here.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
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

// ---------------------------------------------------------------- messaging apps

@Composable
private fun Messaging(ui: SettingsUi) {
    Page {
        if (ui.snap.chatApps == null) { NoAnswer(); Waiting(ui); return@Page }
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
                    Box(Modifier.size(8.dp).background(if (c.on) Mts.BrandGreen else Mts.Line, CircleShape))
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
        Text(key, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.size(90.dp, 20.dp))
        Text(value, style = HqType.BodySm, color = color)
    }
}

// ---------------------------------------------------------------- Connect Claude

@Composable
private fun Claude(ui: SettingsUi) {
    val c = ui.snap.connector
    Page {
        if (c == null) { NoAnswer(); Waiting(ui); return@Page }
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
            Text(SettingsWords.connectorStatus(c, ui.now), style = HqType.BodyBold, color = Mts.Ink)
        }
        Label("Connected")
        val grants = SettingsWords.grants(c, ui.now)
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
        if (d == null) { NoAnswer(); Waiting(ui); return@Page }
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
        if (b == null) { NoAnswer(); Waiting(ui); return@Page }
        val rows = SettingsWords.budgetRows(b)
        Box(Modifier.padding(top = 8.dp)) { if (rows.isEmpty()) Notice("No spending limit is set.", HqIcons.Info) else Facts(rows) }
        Text("Changing a limit, and the full usage page, are in the web app for now.", style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
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
                FactRow("AgentHQ", "hub · build ${ui.build}"),
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
