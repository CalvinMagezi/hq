package example.hq.spike.ui.firstrun

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.LocalNotif
import example.hq.spike.NotifAccess
import example.hq.spike.NotifFactsNow
import example.hq.spike.NotifOrigin
import example.hq.spike.NotifStep
import example.hq.spike.NotifUi
import example.hq.spike.R
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private const val ON_SHOWN_MS = 1000L

/** Without an activity behind it (a preview or a test) the phone is taken to have notifications off and never asked. */
private val NoneNotif = NotifUi(NotifFactsNow(sdk = 34, granted = false, appEnabled = true, needsImportance = 4, rationale = false, asked = false), turnOn = {})

/** The first-run Notifications step: the ask, the example, and the frame for owners who pass or are refused. */
@Composable
fun NotifyStep(ui: FirstRunUi, actions: FirstRunActions) {
    val notif = LocalNotif.current ?: NoneNotif
    val f = notif.facts
    val step = NotifAccess.next(f)
    var onShown by remember { mutableStateOf(false) }
    LaunchedEffect(f.needsOn) {
        if (f.needsOn) {
            onShown = true
            kotlinx.coroutines.delay(ON_SHOWN_MS)
            actions.onNotifNext()
        }
    }
    if (onShown) {
        Text(stringResource(R.string.notify_is_on), style = HqType.H2, color = Mts.BrandGreen)
        return
    }
    if (ui.notifOff) NotifOffFrame(f, step, notif, actions) else AskFrame(step, notif, actions)
}

@Composable
private fun AskFrame(step: NotifStep, notif: NotifUi, actions: FirstRunActions) {
    Heading(stringResource(R.string.notify_title), stringResource(R.string.notify_body))
    ExampleCard()
    TurnOnPill(step, notif, green = true)
    OutlinePill(stringResource(R.string.notify_not_now), onClick = actions.onNotifNotNow)
    TextAction(stringResource(R.string.notify_back), onClick = actions.onBack)
}

@Composable
private fun NotifOffFrame(f: NotifFactsNow, step: NotifStep, notif: NotifUi, actions: FirstRunActions) {
    Text(stringResource(R.string.notify_off_title), style = HqType.H2, color = Mts.Ink)
    val body = when {
        f.sdk < 33 -> R.string.notify_old_android_body
        step == NotifStep.OpenApp -> R.string.notify_no_reason_body
        else -> R.string.notify_off_body
    }
    Text(stringResource(body), style = HqType.Body, color = Mts.InkMuted)
    if (body == R.string.notify_off_body) {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for (line in listOf(R.string.notify_off_line_helper, R.string.notify_off_line_tasks, R.string.notify_off_line_updates)) {
                Text(stringResource(line), style = HqType.Body, color = Mts.Ink)
            }
        }
        Text(stringResource(R.string.notify_off_hint), style = HqType.BodySm, color = Mts.InkMuted)
    }
    GreenPill(stringResource(R.string.notify_continue), onClick = actions.onNotifNext)
    if (step != NotifStep.Done) {
        val label = if (step == NotifStep.AskAndroid) R.string.notify_ask_again else turnOnLabel(step)
        OutlinePill(stringResource(label), onClick = { notif.turnOn(NotifOrigin.FirstRun) })
    }
    TextAction(stringResource(R.string.notify_back), onClick = actions.onBack)
}

@Composable
private fun TurnOnPill(step: NotifStep, notif: NotifUi, green: Boolean) {
    val label = stringResource(turnOnLabel(step))
    if (green) GreenPill(label, onClick = { notif.turnOn(NotifOrigin.FirstRun) })
    else OutlinePill(label, onClick = { notif.turnOn(NotifOrigin.FirstRun) })
}

private fun turnOnLabel(step: NotifStep): Int = when (step) {
    NotifStep.OpenApp -> R.string.notif_open_android_settings
    NotifStep.OpenChannel -> R.string.notify_turn_on_needs
    else -> R.string.notify_allow
}

/** A picture of the notice, drawn by the app: it is not a real notification. Read once by TalkBack, as one description. */
@Composable
private fun ExampleCard() {
    val description = stringResource(R.string.notify_example_description)
    Column(
        Modifier.fillMaxWidth().clearAndSetSemantics { contentDescription = description }
            .clip(RoundedCornerShape(16.dp)).background(Mts.Surface100).border(1.dp, Mts.Line, RoundedCornerShape(16.dp)).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(stringResource(R.string.notify_example_header), style = HqType.Caption, color = Mts.InkMuted)
            Text(stringResource(R.string.notify_example_label), style = HqType.Caption, color = Mts.InkMuted)
        }
        Text(stringResource(R.string.notify_example_title), style = HqType.BodyBold, color = Mts.Ink)
        Text(stringResource(R.string.notify_example_body), style = HqType.BodySm, color = Mts.InkMuted)
        Text(stringResource(R.string.notify_example_action), style = HqType.BodySmBold, color = Mts.BrandBlue)
    }
}

/** On Ready to start: while notifications are off, an amber card says questions will wait, with Turn on first. */
@Composable
fun StartNotifCard() {
    val notif = LocalNotif.current ?: return
    if (notif.facts.needsOn) return
    val step = NotifAccess.next(notif.facts)
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Mts.Surface100).border(1.dp, Mts.Amber, RoundedCornerShape(12.dp)).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(stringResource(R.string.notify_ready_off_title), style = HqType.BodyBold, color = Mts.Ink)
        Text(stringResource(R.string.notify_ready_off_body), style = HqType.BodySm, color = Mts.InkMuted)
        OutlinePill(stringResource(R.string.notify_turn_on), onClick = { if (step != NotifStep.Done) notif.turnOn(NotifOrigin.FirstRun) })
    }
}

/** The sheet an upgrader sees once: the same ask as first run, but closing it is not a refusal. */
@Composable
fun NotifUpgraderSheet(onAllow: () -> Unit, onClose: () -> Unit) {
    androidx.compose.ui.window.Dialog(onDismissRequest = onClose) {
        Column(
            Modifier.clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(stringResource(R.string.notify_title), style = HqType.H2, color = Mts.Ink)
            Text(stringResource(R.string.notify_body), style = HqType.BodySm, color = Mts.InkMuted)
            GreenPill(stringResource(R.string.notify_allow), onClick = onAllow)
            OutlinePill(stringResource(R.string.notify_not_now), onClick = onClose)
        }
    }
}
