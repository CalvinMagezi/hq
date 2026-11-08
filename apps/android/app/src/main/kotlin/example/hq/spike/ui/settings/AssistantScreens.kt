package example.hq.spike.ui.settings

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.os.Build
import android.os.SystemClock
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.window.Dialog
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
import example.hq.spike.voice.MicAsk
import example.hq.spike.voice.MicFact
import example.hq.spike.voice.MicOutcome
import example.hq.spike.voice.VoiceModels

/** What Android says about HQ as the assistant right now, read again each time the screen comes back from Android's settings. */
internal data class AssistantFacts(
    val available: Boolean,
    val held: Boolean,
    val secure: Boolean,
    val lockAllowed: Boolean,
    val mic: MicFact = MicFact.NotAsked,
) {
    companion object {
        fun read(c: Context) = AssistantFacts(
            AssistantRole.available(c), AssistantRole.held(c), AssistantRole.deviceSecure(c), Updater.lockScreenAssistAllowed(c), AssistantRole.micState(c),
        )
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
internal fun AssistantTapSteps(onOpened: () -> Unit = {}, onClose: () -> Unit) {
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
        CardButton(w.s(R.string.assist_open_android), true) {
            failed = !PhoneIntents.startFirst(c, AssistantRole.settingsTries(c))
            if (!failed) onOpened()
        }
        CardButton(w.s(R.string.confirm_not_now), false, onClose)
    }
}

/**
 * The Phone assistant page: three facts, the way in, and the privacy line. "Make HQ the assistant" opens the steps over
 * the page, and coming back from Android's settings says whether it worked. The microphone can always be asked for
 * again from here.
 */
@Composable
internal fun AssistantPage(ui: SettingsUi, actions: SettingsActions) {
    val c = LocalContext.current
    val w = rememberWords()
    val f = rememberAssistantFacts()
    val models = remember { VoiceModels(c.applicationContext) }
    val mic = f.mic
    var steps by remember { mutableStateOf(false) }
    var wentToSettings by remember { mutableStateOf(false) }
    val activity = c.activity()
    val askMic = remember(activity) { MicAskState(activity) }
    val micDialog = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { askMic.answered(it) }
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
                FactRow(w.s(R.string.assist_fact_mic), AssistantWords.micLine(w, mic), good = mic == MicFact.Allowed),
                FactRow(w.s(R.string.assist_fact_model), model, opens = SettingsPage.Speech),
            ),
            actions.onPage,
        )
        MicButton(w, AssistantWords.micAction(mic, askMic.blocked), onAsk = { askMic.ask { micDialog.launch(it) } }) {
            PhoneIntents.startFirst(c, listOf(AssistantRole.appInfo(c)))
        }
        AssistantWords.afterSettings(wentToSettings, f.held)?.let { res ->
            Box(Modifier.padding(top = 12.dp)) { Notice(w.s(res), if (f.held) HqIcons.Check else HqIcons.Info, warn = !f.held) }
        }
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
            if (!f.held) CardButton(w.s(R.string.assist_make), true) { steps = true }
            CardButton(w.s(R.string.assist_open_chat), false) { openChat(c) }
        }
        if (steps) {
            Dialog(onDismissRequest = { steps = false }) {
                Column(Modifier.clip(RoundedCornerShape(16.dp)).background(Mts.Surface100)) {
                    AssistantTapSteps(onOpened = { wentToSettings = true; steps = false }) { steps = false }
                }
            }
        }
        if (f.held) Text(w.s(R.string.assist_stop_note), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, end = 20.dp))
        Text(w.s(R.string.assist_samsung_note), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
        Text(if (server != null) w.s(R.string.assist_privacy_server, server) else w.s(R.string.assist_privacy), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp, bottom = 16.dp))
        LockCard(f, w)
    }
}

/** Under the facts: Android's dialog, asking again, or App info once Android stopped asking. Nothing when allowed. */
@Composable
private fun MicButton(w: Words, action: MicAction, onAsk: () -> Unit, onAppInfo: () -> Unit) {
    if (action == MicAction.None) return
    if (action == MicAction.AppInfo) Box(Modifier.padding(top = 12.dp)) { Notice(w.s(R.string.assist_mic_blocked), HqIcons.Warning, warn = true) }
    Column(Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp)) {
        when (action) {
            MicAction.Allow -> CardButton(w.s(R.string.assist_mic_allow), false, onAsk)
            MicAction.AskAgain -> CardButton(w.s(R.string.voice_ask_again), false, onAsk)
            else -> CardButton(w.s(R.string.assist_open_app_info), false, onAppInfo)
        }
    }
}

/** Android's microphone dialog from the Phone assistant page, read the same way the voice sheet reads it. */
internal class MicAskState(private val activity: Activity?) {
    /** Android answered without showing its dialog, so only App info can turn the microphone on. */
    var blocked by mutableStateOf(false)
    private var rationaleBefore = false
    private var askedAt = 0L

    fun ask(launch: (String) -> Unit) {
        val a = activity ?: return
        rationaleBefore = a.shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO)
        askedAt = SystemClock.elapsedRealtime()
        AssistantRole.markMicAsked(a)
        launch(Manifest.permission.RECORD_AUDIO)
    }

    fun answered(granted: Boolean) {
        val after = activity?.shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO) ?: false
        blocked = MicAsk.outcome(granted, rationaleBefore, after, SystemClock.elapsedRealtime() - askedAt) == MicOutcome.Blocked
    }
}

private tailrec fun Context.activity(): Activity? = when (this) {
    is Activity -> this
    is ContextWrapper -> baseContext.activity()
    else -> null
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
