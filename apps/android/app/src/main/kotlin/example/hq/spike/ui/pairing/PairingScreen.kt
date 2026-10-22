package example.hq.spike.ui.pairing

import android.Manifest
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.PowerManager
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.runtime.collectAsState
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import example.hq.spike.bridge.BridgePeer
import example.hq.spike.bridge.BridgeState
import example.hq.spike.bridge.JoinState
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.delay

private const val PEERS_REFRESH_MS = 15_000L
private const val TICK_MS = 1_000L
private val QR_SIDE = 280.dp

/** The pairing screens: what this phone shows, what it scans and who is paired. */
@Composable
fun PairingHost(vm: PairingViewModel, onClose: () -> Unit) {
    val ui by vm.ui.collectAsStateWithLifecycle()
    val bridge by vm.bridge.collectAsStateWithLifecycle()
    BackHandler { if (ui.screen == PairingScreen.Home) onClose() else vm.home() }
    Box(Modifier.fillMaxSize().background(Mts.Surface000).statusBarsPadding().navigationBarsPadding()) {
        Column(Modifier.fillMaxSize()) {
            TopBar(title(ui.screen)) { if (ui.screen == PairingScreen.Home) onClose() else vm.home() }
            when (ui.screen) {
                PairingScreen.Home -> Home(vm, ui, bridge)
                PairingScreen.Show -> Show(vm, ui, bridge)
                PairingScreen.Scan -> Scan(vm, ui)
                PairingScreen.Paste -> Paste(vm, ui)
                PairingScreen.Confirm -> Confirm(vm, ui)
                PairingScreen.Joining -> Joining(vm, bridge)
            }
        }
        ui.askRemove?.let { RemoveDialog(it, { vm.askRemove(null) }, vm::confirmRemove) }
        ui.line?.let { Notice(it, Modifier.align(Alignment.BottomCenter), vm::dismissLine) }
    }
}

private fun title(s: PairingScreen) = when (s) {
    PairingScreen.Home -> PairingWords.TITLE
    PairingScreen.Show -> PairingWords.SHOW_TITLE
    PairingScreen.Scan -> PairingWords.SCAN_TITLE
    PairingScreen.Paste -> PairingWords.PASTE_TITLE
    PairingScreen.Confirm -> PairingWords.CONFIRM_TITLE
    PairingScreen.Joining -> "Pairing"
}

@Composable
internal fun TopBar(title: String, onBack: () -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 4.dp, end = 16.dp), verticalAlignment = Alignment.CenterVertically) {
        IconAction(HqIcons.ArrowBack, "Back", iconSize = 24.dp, onClick = onBack)
        Text(title, style = HqType.ScreenTitle, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 4.dp))
    }
}

@Composable
internal fun Page(content: @Composable () -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 24.dp)) { content() }
}

@Composable
internal fun Body(text: String, modifier: Modifier = Modifier) = Text(text, style = HqType.Body, color = Mts.InkMuted, modifier = modifier)

@Composable
internal fun Label(text: String) = Text(text.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(top = 24.dp, bottom = 8.dp))

@Composable
internal fun PillButton(text: String, primary: Boolean = true, enabled: Boolean = true, tag: String = text, onClick: () -> Unit) {
    val background = if (primary) Mts.BrandGreen else Mts.Surface200
    val ink = if (primary) Mts.OnGreen else Mts.Ink
    Box(
        Modifier.testTag(tag).fillMaxWidth().heightIn(min = 52.dp).clip(RoundedCornerShape(26.dp))
            .background(if (enabled) background else Mts.Surface200)
            .clickable(enabled = enabled, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { Text(text, style = HqType.BodyBold, color = if (enabled) ink else Mts.Line) }
}

@Composable
internal fun Notice(text: String, modifier: Modifier, onDismiss: () -> Unit) {
    LaunchedEffect(text) { delay(NOTICE_MS); onDismiss() }
    Row(modifier.padding(8.dp).fillMaxWidth().heightIn(min = 48.dp).background(Mts.Ink, RoundedCornerShape(6.dp)).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(text, style = HqType.BodySm, color = Mts.Surface100)
    }
}

private const val NOTICE_MS = 5_000L

// ---------------------------------------------------------------- home

@Composable
private fun Home(vm: PairingViewModel, ui: PairingUi, bridge: BridgeState) {
    LaunchedEffect(bridge.running) {
        while (bridge.running) { vm.refresh(); delay(PEERS_REFRESH_MS) }
    }
    Page {
        Body(PairingWords.INTRO)
        Row(Modifier.fillMaxWidth().padding(top = 20.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Paired devices", style = HqType.BodyBold, color = Mts.Ink)
                Text(PairingWords.switchLine(bridge), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.testTag("switch-line"))
            }
            Switch(
                checked = bridge.enabled, onCheckedChange = vm::setPairedDevices, modifier = Modifier.testTag("paired-switch"),
                colors = SwitchDefaults.colors(checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen),
            )
        }
        PairingWords.relayLine(bridge.relay)?.let { Text(it, style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(top = 4.dp)) }
        BatteryNotice(bridge)
        Spacer(Modifier.height(20.dp))
        PillButton("Show a code", tag = "show-code") { vm.showCode() }
        Spacer(Modifier.height(12.dp))
        PillButton("Scan a code", primary = false, tag = "scan-code") { vm.go(PairingScreen.Scan) }
        Spacer(Modifier.height(12.dp))
        PillButton("Paste a link", primary = false, tag = "paste-link") { vm.go(PairingScreen.Paste) }
        Spacer(Modifier.height(12.dp))
        val context = LocalContext.current
        PillButton("Connect Claude", primary = false, tag = "connect-claude") { context.startActivity(Intent(context, example.hq.spike.ui.connect.ConnectClaudeActivity::class.java)) }
        Label("Paired with this phone")
        val peers = bridge.peers
        when {
            peers == null -> Body(if (bridge.enabled) "Looking…" else "Turn paired devices on to see them.")
            peers.isEmpty() -> Body("No device is paired.")
            else -> peers.forEach { PeerRow(it) { vm.askRemove(it) } }
        }
    }
}

/** Android may pause a sleeping app; the owner can allow this one to run freely (the same page as Settings, Keep running). */
@Composable
private fun BatteryNotice(bridge: BridgeState) {
    val context = LocalContext.current
    var ignoring by remember { mutableStateOf(true) }
    LifecycleResumeEffect(Unit) {
        ignoring = context.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(context.packageName)
        onPauseOrDispose {}
    }
    if (!PairingWords.showBatteryNotice(bridge, ignoring)) return
    Column(Modifier.fillMaxWidth().padding(top = 16.dp)) {
        Text(PairingWords.BATTERY_NOTICE, style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.testTag("battery-notice"))
        TextButton(onClick = { context.startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)) }) {
            Text(PairingWords.BATTERY_BUTTON, color = Mts.Ink, style = HqType.BodySmBold)
        }
    }
}

@Composable
private fun PeerRow(peer: BridgePeer, onRemove: () -> Unit) {
    val now = System.currentTimeMillis() / MS_PER_SEC
    Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).testTag("peer-${peer.id.take(8)}"), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
            Text(peer.name.ifBlank { "Unnamed device" }, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(PairingWords.peerLine(peer, now), style = HqType.BodySm, color = Mts.InkMuted)
        }
        TextButton(onClick = onRemove) { Text("Remove", color = Mts.DangerText, style = HqType.BodySmBold) }
    }
}

private const val MS_PER_SEC = 1_000L

@Composable
private fun RemoveDialog(peer: BridgePeer, onCancel: () -> Unit, onConfirm: () -> Unit) {
    AlertDialog(
        onDismissRequest = onCancel,
        containerColor = Mts.Surface100,
        title = { Text(PairingWords.removeTitle(peer), style = HqType.ScreenTitle, color = Mts.Ink) },
        text = { Text(PairingWords.REMOVE_BODY, style = HqType.Body, color = Mts.InkMuted) },
        confirmButton = { TextButton(onClick = onConfirm) { Text("Remove", color = Mts.DangerText, style = HqType.BodySmBold) } },
        dismissButton = { TextButton(onClick = onCancel) { Text("Cancel", color = Mts.Ink, style = HqType.BodySmBold) } },
    )
}

// ---------------------------------------------------------------- this phone shows a code

@Composable
private fun Show(vm: PairingViewModel, ui: PairingUi, bridge: BridgeState) {
    val context = LocalContext.current
    var now by remember { mutableStateOf(System.currentTimeMillis()) }
    LaunchedEffect(bridge.ticket) { while (true) { now = System.currentTimeMillis(); delay(TICK_MS) } }
    Page {
        val ticket = bridge.ticket
        val request = bridge.request
        val end = bridge.ticketEnd
        when {
            request != null -> {
                Body(PairingWords.wants(request.name, request.joiner), Modifier.testTag("wants"))
                DigitsField(ui.typed, vm::typeDigits)
                Spacer(Modifier.height(16.dp))
                PillButton("Pair", enabled = ui.typed.length == PairingWords.DIGITS_LENGTH, tag = "pair-digits") { vm.submitDigits() }
            }
            ticket != null -> {
                Body(PairingWords.SHOW_HELP)
                Spacer(Modifier.height(16.dp))
                QrImage(ticket.link)
                Text(PairingWords.countdown(ticket.endsAt - now), style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.fillMaxWidth().padding(top = 12.dp).testTag("countdown"), textAlign = TextAlign.Center)
                Spacer(Modifier.height(12.dp))
                PillButton("Copy the link", primary = false, tag = "copy-link") { copy(context, ticket.link) }
                Spacer(Modifier.height(12.dp))
                PillButton("Cancel", primary = false, tag = "cancel-code") { vm.home() }
            }
            end != null -> {
                Body(PairingWords.ended(end), Modifier.testTag("ended"))
                Spacer(Modifier.height(16.dp))
                PillButton("Show a new code", tag = "show-new") { vm.showCode() }
                Spacer(Modifier.height(12.dp))
                PillButton("Done", primary = false, tag = "done") { vm.home() }
            }
            else -> Body(if (bridge.running) "Making a code…" else "Starting…")
        }
    }
}

@Composable
private fun QrImage(link: String) {
    val bitmap = remember(link) { QrCode.bitmap(link).asImageBitmap() }
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Image(
            bitmap, contentDescription = "Pairing code", filterQuality = FilterQuality.None,
            modifier = Modifier.size(QR_SIDE).background(Color.White).padding(8.dp).testTag("qr"),
        )
    }
}

private fun copy(context: Context, link: String) {
    val manager = context.getSystemService(ClipboardManager::class.java)
    manager.setPrimaryClip(ClipData.newPlainText("HQ pairing link", link).apply {
        // Keep the link out of the clipboard preview and history where the system allows.
        description.extras = android.os.PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
    })
}

/** Six boxes' worth of digits in one field; the person types what the other device shows. */
@Composable
private fun DigitsField(value: String, onChange: (String) -> Unit) {
    BasicTextField(
        value = value, onValueChange = onChange, singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
        textStyle = HqType.H2.copy(color = Mts.Ink, letterSpacing = 0.4.em, textAlign = TextAlign.Center, fontFamily = FontFamily.Monospace, fontSize = 32.sp),
        cursorBrush = SolidColor(Mts.BrandGreen),
        modifier = Modifier.fillMaxWidth().padding(top = 16.dp).testTag("digits"),
        decorationBox = { inner ->
            Box(Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(RoundedCornerShape(12.dp)).background(Mts.Surface100).border(1.dp, Mts.Hairline, RoundedCornerShape(12.dp)), contentAlignment = Alignment.Center) { inner() }
        },
    )
}

// ---------------------------------------------------------------- this phone scans a code

@Composable
private fun Scan(vm: PairingViewModel, ui: PairingUi) {
    val context = LocalContext.current
    var granted by remember { mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    var asked by remember { mutableStateOf(false) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it; asked = true }
    LaunchedEffect(Unit) { if (!granted) ask.launch(Manifest.permission.CAMERA) }
    Page {
        if (granted) {
            Body(PairingWords.SCAN_HELP)
            Spacer(Modifier.height(16.dp))
            QrScanner(Modifier.fillMaxWidth().height(360.dp).clip(RoundedCornerShape(12.dp)).testTag("scanner")) { vm.joinText(it) }
        } else {
            Body(if (asked) PairingWords.CAMERA_NEEDED else "Asking for the camera…")
            Spacer(Modifier.height(16.dp))
            PillButton("Paste a link instead", tag = "paste-instead") { vm.go(PairingScreen.Paste) }
        }
    }
}

// ---------------------------------------------------------------- paste, confirm, joining

@Composable
private fun Paste(vm: PairingViewModel, ui: PairingUi) {
    Page {
        Body("Paste the link from the other device.")
        BasicTextField(
            value = ui.link, onValueChange = vm::editLink, textStyle = HqType.BodySm.copy(color = Mts.Ink, fontFamily = FontFamily.Monospace),
            cursorBrush = SolidColor(Mts.BrandGreen), maxLines = 6,
            modifier = Modifier.fillMaxWidth().padding(top = 16.dp).testTag("link-field"),
            decorationBox = { inner ->
                Box(Modifier.fillMaxWidth().heightIn(min = 96.dp).clip(RoundedCornerShape(12.dp)).background(Mts.Surface100).padding(12.dp)) {
                    if (ui.link.isEmpty()) Text(PairingWords.PASTE_HINT, style = HqType.BodySm, color = Mts.Line)
                    inner()
                }
            },
        )
        Spacer(Modifier.height(16.dp))
        PillButton("Pair", enabled = ui.link.isNotBlank(), tag = "pair-link") { vm.joinText(ui.link) }
    }
}

@Composable
private fun Confirm(vm: PairingViewModel, ui: PairingUi) {
    Page {
        Body(PairingWords.CONFIRM_BODY)
        Spacer(Modifier.height(24.dp))
        PillButton("Pair", tag = "confirm-pair") { vm.startJoin(ui.link) }
        Spacer(Modifier.height(12.dp))
        PillButton("Cancel", primary = false, tag = "confirm-cancel") { vm.home() }
    }
}

@Composable
private fun Joining(vm: PairingViewModel, bridge: BridgeState) {
    Page {
        when (val join = bridge.join) {
            JoinState.Idle, JoinState.Connecting -> Body(if (bridge.running) "Connecting to the other device…" else "Starting…")
            is JoinState.ShowDigits -> {
                Body(PairingWords.DIGITS_HELP)
                Text(join.value, style = HqType.H2.copy(fontSize = 44.sp, letterSpacing = 0.3.em, fontFamily = FontFamily.Monospace), color = Mts.Ink, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(vertical = 32.dp).testTag("shown-digits"))
            }
            is JoinState.Done -> {
                Body(PairingWords.joined(join.name, join.id), Modifier.testTag("joined"))
                Spacer(Modifier.height(16.dp))
                PillButton("Done", tag = "joined-done") { vm.home() }
            }
            is JoinState.Failed -> {
                Body(join.why, Modifier.testTag("join-failed"))
                Spacer(Modifier.height(16.dp))
                PillButton("Back", tag = "join-back") { vm.home() }
            }
        }
    }
}
