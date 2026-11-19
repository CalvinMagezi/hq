package example.hq.spike.ui.settings

import example.hq.spike.ui.shell.AttentionKind
import example.hq.spike.ui.shell.AttentionOrInline
import example.hq.spike.ui.shell.SnackbarRow
import example.hq.spike.ui.shell.rememberAttentionKey
import example.hq.spike.R
import example.hq.spike.ui.firstrun.FirstRunWords
import example.hq.spike.ui.firstrun.StoredKeysPage
import androidx.activity.compose.BackHandler
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.heading
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
import example.hq.spike.update.Updater
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

// Notifications, silences and the server HQ section of Settings. Split from SettingsScreens.kt; the code is as it was.
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
        ui.silences?.let { SilenceSection(it, ui.silenceBusy, actions) }
        ServerHqSection(n.server, ui, actions)
    }
    if (asking) DetailsDialog(onKeep = { asking = false }, onShow = { asking = false; actions.onDetails(true) })
}

private const val PAUSE_HOUR_MINUTES = 60
private const val PAUSE_NIGHT_MINUTES = 8 * 60

private val SILENCE_TOPICS = listOf(
    Triple("helpers", R.string.topic_helpers, R.string.topic_helpers_about),
    // Shown only when HQ lists this topic; an HQ that folds finishes into "helpers" has no row for it.
    Triple("helpers_finishing", R.string.topic_helpers_finishing, R.string.topic_helpers_finishing_about),
    Triple("tasks", R.string.topic_tasks, R.string.topic_tasks_about),
    Triple("tips", R.string.topic_tips, R.string.topic_tips_about),
    Triple("upkeep", R.string.topic_upkeep, R.string.topic_upkeep_about),
    Triple("email", R.string.topic_email, R.string.topic_email_about),
)

/** Which topics may interrupt the owner. Kept on HQ, so it holds for the web as well. Each switch reads "notifications on". */
@Composable
private fun SilenceSection(silences: Silences, busy: Boolean, actions: SettingsActions) {
    val w = example.hq.spike.ui.shell.rememberWords()
    val paused = SettingsWords.pauseLine(w, silences.pause)
    Label(stringResource(R.string.silence_title))
    Text(stringResource(R.string.silence_intro), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 4.dp, end = 20.dp, bottom = 8.dp))
    Group {
        Column(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp)) {
            Text(stringResource(R.string.silence_pause), style = HqType.BodyBold, color = Mts.Ink)
            if (paused != null) Text(paused, style = HqType.BodySm, color = Mts.InkMuted)
            Row {
                if (paused != null) DialogButton(stringResource(R.string.silence_resume)) { if (!busy) actions.onResume() }
                else {
                    DialogButton(stringResource(R.string.silence_pause_hour)) { if (!busy) actions.onPause(PAUSE_HOUR_MINUTES) }
                    DialogButton(stringResource(R.string.silence_pause_night)) { if (!busy) actions.onPause(PAUSE_NIGHT_MINUTES) }
                }
            }
        }
        for ((topic, name, about) in SILENCE_TOPICS) {
            val row = silences.topics.firstOrNull { it.topic == topic } ?: continue
            Divider()
            Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(name), style = HqType.BodyBold, color = Mts.Ink)
                    Text(SettingsWords.silenceLine(w, row) + " \u00b7 " + stringResource(about), style = HqType.BodySm, color = Mts.InkMuted)
                }
                Switch(
                    checked = !row.silenced, onCheckedChange = { on -> actions.onSilenceTopic(topic, !on) }, enabled = !busy,
                    colors = SwitchDefaults.colors(
                        checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen, checkedBorderColor = Mts.BrandGreen,
                        uncheckedTrackColor = Mts.Surface200, uncheckedThumbColor = Mts.Line, uncheckedBorderColor = Mts.Line,
                    ),
                )
            }
        }
    }
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
internal fun InfoCard(title: String, text: String) {
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
    Label(stringResource(R.string.settings_server_hq_title))
    if (server.connected) {
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_signed_in), style = HqType.BodyBold, color = Mts.Ink)
                    Text(server.address.orEmpty(), style = HqType.BodySm, color = Mts.InkMuted)
                }
                DialogButton(stringResource(R.string.settings_disconnect)) { actions.onAskServerDisconnect(true) }
            }
        }
        Text(
            stringResource(R.string.settings_questions_from_coding_agents),
            style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp),
        )
        return
    }
    if (server.keyLost) NewPhoneCard()
    else if (server.signedOut) Box(Modifier.padding(top = 4.dp)) { NoticeCard(stringResource(R.string.settings_server_signed_out_title), stringResource(R.string.settings_server_signed_out_body, server.address.orEmpty()), true) }
    val busy = ui.busy == SERVER_BUSY
    var address by remember { androidx.compose.runtime.mutableStateOf(server.address.orEmpty()) }
    var code by remember { androidx.compose.runtime.mutableStateOf("") }
    Column(Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.settings_if_your_hq_runs), style = HqType.BodySm, color = Mts.InkMuted)
        ServerField(stringResource(R.string.sheet_address_label), address, "https://hq.example.com", busy, androidx.compose.ui.text.input.KeyboardType.Uri) { address = it }
        ServerField(stringResource(R.string.signin_code_label), code, "XXXXX-XXXXX", busy, androidx.compose.ui.text.input.KeyboardType.Ascii) { code = it }
        ui.serverError?.let { Text(it, style = HqType.BodySm, color = Mts.DangerText) }
        GreenButton(stringResource(if (busy) R.string.settings_signing_in_dots else R.string.settings_sign_in), enabled = !busy && address.isNotBlank() && code.isNotBlank(), busy = busy) { actions.onServerConnect(address, code) }
    }
}

/** A server HQ's owner on a new phone: the notes are on the server, only this phone's sign-in stayed behind. */
@Composable
private fun NewPhoneCard() {
    var hidden by remember { androidx.compose.runtime.mutableStateOf(false) }
    if (hidden) return
    val context = androidx.compose.ui.platform.LocalContext.current
    Box(Modifier.padding(top = 4.dp)) { NoticeCard(stringResource(R.string.server_new_phone_title), stringResource(R.string.server_new_phone_body), true) }
    Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.server_new_phone_how), style = HqType.BodySm, color = Mts.InkMuted)
        GreenButton(stringResource(R.string.server_new_phone_scan)) {
            context.startActivity(android.content.Intent(context, example.hq.spike.ui.pairing.PairingActivity::class.java).putExtra(example.hq.spike.ui.pairing.PairingActivity.EXTRA_SCREEN, "scan"))
        }
        TextLink(stringResource(R.string.server_new_phone_later), Mts.Ink) { hidden = true }
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
