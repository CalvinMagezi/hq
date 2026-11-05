package example.hq.spike.ui.settings

import android.content.Context
import android.os.Build
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import example.hq.spike.R
import example.hq.spike.ServerHq
import example.hq.spike.download.DownloadRules
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.Words
import example.hq.spike.ui.shell.rememberWords
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import example.hq.spike.update.Updater
import example.hq.spike.voice.AssistantRole
import example.hq.spike.voice.VoiceModels

/** What Android says about HQ as the assistant right now, read again each time the screen comes back from Android's settings. */
internal data class AssistantFacts(val available: Boolean, val held: Boolean, val secure: Boolean, val lockAllowed: Boolean) {
    companion object {
        fun read(c: Context) = AssistantFacts(AssistantRole.available(c), AssistantRole.held(c), AssistantRole.deviceSecure(c), Updater.lockScreenAssistAllowed(c))
    }
}

/** The facts as a state that is re-read on resume. */
@Composable
internal fun rememberAssistantFacts(): AssistantFacts {
    val c = LocalContext.current
    var facts by remember { mutableStateOf(AssistantFacts.read(c)) }
    var tick by remember { mutableIntStateOf(0) }
    val owner = LocalLifecycleOwner.current
    DisposableEffect(owner) {
        val obs = LifecycleEventObserver { _, e -> if (e == Lifecycle.Event.ON_RESUME) { facts = AssistantFacts.read(c); tick++ } }
        owner.lifecycle.addObserver(obs)
        onDispose { owner.lifecycle.removeObserver(obs) }
    }
    return facts
}

/** The "what to tap" sheet: the same in first run and in Settings. Falls back to the sentence when no Android page opens. */
@Composable
internal fun AssistantTapSteps(onClose: () -> Unit) {
    val c = LocalContext.current
    val w = rememberWords()
    var failed by remember { mutableStateOf(false) }
    Column(Modifier.padding(20.dp)) {
        Text(w.s(R.string.assist_sheet_title), style = HqType.BodyBold, color = Mts.Ink)
        Text(w.s(AssistantWords.firstStep(Build.VERSION.SDK_INT)), style = HqType.Body, color = Mts.Ink, modifier = Modifier.padding(top = 8.dp))
        Text(w.s(R.string.assist_step_choose), style = HqType.Body, color = Mts.Ink)
        Text(w.s(R.string.assist_step_ok), style = HqType.Body, color = Mts.Ink)
        Text(w.s(R.string.assist_sheet_note), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(top = 8.dp))
        if (failed) Text(w.s(R.string.assist_no_page), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(top = 8.dp))
        CardButton(w.s(R.string.assist_open_android), true) { failed = !PhoneIntents.startFirst(c, AssistantRole.settingsTries(c)) }
        CardButton(w.s(R.string.confirm_not_now), false, onClose)
    }
}

/** The Phone assistant page: three facts, the way in, and the privacy line. */
@Composable
internal fun AssistantPage(ui: SettingsUi, actions: SettingsActions) {
    val c = LocalContext.current
    val w = rememberWords()
    val f = rememberAssistantFacts()
    val models = remember { VoiceModels(c.applicationContext) }
    val mic = AssistantRole.micState(c)
    var steps by remember { mutableStateOf(false) }
    val server = ServerHq.address(c)?.let { runCatching { java.net.URI(it).host }.getOrNull() }
    Page {
        if (!f.available) {
            Notice(w.s(R.string.assist_na_body), HqIcons.Lock)
            return@Page
        }
        Text(w.s(R.string.assist_intro), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
        Label(w.s(R.string.assist_group_right_now))
        val modelBytes = VoiceModels.SIZE_BYTES
        val model = when {
            models.ready() -> w.s(R.string.assist_model_on, w.s(R.string.dl_mb, DownloadRules.sizeMb(models.onPhoneBytes())))
            else -> w.s(R.string.assist_model_missing, w.s(R.string.dl_mb, DownloadRules.sizeMb(modelBytes)))
        }
        Facts(
            listOf(
                FactRow(w.s(R.string.assist_fact_app), w.s(if (f.held) R.string.assist_app_hq else R.string.assist_app_other)),
                FactRow(w.s(R.string.assist_fact_mic), AssistantWords.micLine(w, mic), good = mic == example.hq.spike.voice.MicFact.Allowed),
                FactRow(w.s(R.string.assist_fact_model), model, opens = SettingsPage.Speech),
            ),
            actions.onPage,
        )
        if (mic == example.hq.spike.voice.MicFact.Off) Notice(w.s(R.string.assist_mic_taps), HqIcons.Warning, warn = true)
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
            if (!f.held) CardButton(w.s(R.string.assist_make), true) { steps = true }
            CardButton(w.s(R.string.assist_open_chat), false) { openChat(c) }
        }
        if (steps) AssistantTapSteps { steps = false }
        if (f.held) Text(w.s(R.string.assist_stop_note), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, end = 20.dp))
        Text(w.s(R.string.assist_samsung_note), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
        Text(if (server != null) w.s(R.string.assist_privacy_server, server) else w.s(R.string.assist_privacy), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp, bottom = 16.dp))
        LockCard(f, w)
    }
}

private fun openChat(c: Context) {
    c.startActivity(android.content.Intent(c, example.hq.spike.ShellActivity::class.java).setData(android.net.Uri.parse("hq://chat")).putExtra(example.hq.spike.ui.shell.Links.EXTRA, "hq://chat").addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK or android.content.Intent.FLAG_ACTIVITY_SINGLE_TOP))
}

/** What happens at a locked press: the screen-lock nudge when there is no lock, else the line for this phone. */
@Composable
internal fun LockCard(f: AssistantFacts, w: Words) {
    val c = LocalContext.current
    if (!f.secure) {
        Label(w.s(R.string.assist_nolock_title))
        Text(w.s(R.string.assist_nolock_body), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, end = 20.dp))
        Text(w.s(R.string.assist_nolock_taps), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, end = 20.dp))
        Column(Modifier.padding(16.dp)) { CardButton(w.s(R.string.assist_open_android), false) { PhoneIntents.startFirst(c, AssistantRole.screenLockTries()) } }
    } else {
        Label(w.s(R.string.lock_card_title))
        Text(w.s(if (f.lockAllowed) R.string.lock_card_a_body else R.string.lock_card_b_body), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 16.dp))
    }
}
