package example.hq.spike.voice

import android.Manifest
import android.app.KeyguardManager
import example.hq.spike.measure.Counter
import example.hq.spike.measure.Counters
import android.content.ActivityNotFoundException
import android.content.Context
import android.net.Uri
import android.os.SystemClock
import example.hq.spike.ui.shell.Links
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.remember
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import example.hq.spike.R
import example.hq.spike.download.DlState
import example.hq.spike.download.Downloads
import example.hq.spike.download.Item
import example.hq.spike.download.LargeDownload
import example.hq.spike.download.Op
import example.hq.spike.download.perform
import example.hq.spike.ui.download.DownloadDialogs
import example.hq.spike.ui.download.DownloadRow
import example.hq.spike.ui.shell.Words
import example.hq.spike.ui.shell.rememberWords
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.lifecycleScope
import example.hq.spike.data.HqServices
import example.hq.spike.ui.theme.HqTheme
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/**
 * The sheet holding Home opens. It hears one request, shows the reply as it arrives, and listens again
 * on a tap. Audio is recognised on the phone by [VoiceSession]; only the final words go to HQ's chat.
 */
class VoiceActivity : ComponentActivity() {
    private lateinit var session: VoiceSession
    private val unlocked = androidx.compose.runtime.mutableStateOf(false)
    /** The unlock prompt is up; the sheet stays while it is, since the prompt itself hides the sheet. */
    private var dismissing = false
    /** A press (the first open or a new intent) waits for onResume to start. */
    private var askOnResume = false

    /** App info is open over the sheet; leaving for it must not close the sheet, and coming back re-reads the microphone. */
    private var awayToSettings = false
    private var rationaleBefore = false
    private var askedAt = 0L

    private val askMic = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val after = shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO)
        when (MicAsk.outcome(granted, rationaleBefore, after, SystemClock.elapsedRealtime() - askedAt)) {
            MicOutcome.Granted -> session.listen()
            MicOutcome.Needed -> session.micNeeded()
            MicOutcome.Blocked -> session.micBlocked()
            MicOutcome.Dismissed -> Unit
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(statusBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT), navigationBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT))
        HqServices.ensureHubRunning(this)
        val services = HqServices.get(applicationContext)
        session = VoiceSession(applicationContext, lifecycleScope, services.connection)
        setContent {
            HqTheme {
                val state by session.state.collectAsState()
                val downloads by Downloads.states.collectAsState()
                androidx.compose.runtime.LaunchedEffect(state.phase == VoicePhase.Thinking || state.phase == VoicePhase.LookingUp) {
                    if (state.phase == VoicePhase.Thinking || state.phase == VoicePhase.LookingUp) {
                        window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                        kotlinx.coroutines.delay(SCREEN_ON_MS)
                    }
                    window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                }
                val item = remember { VoiceModels(applicationContext).item() }
                if (unlocked.value) VoiceSheet(state, downloads[item.id] ?: DlState.Ready, item, onOp = { op -> session.engine().perform(op) }, engine = { Downloads.get(item.id) }, onMic = ::micPressed, onClose = ::finish, onSettings = ::openAppInfo, onOpenChat = ::openChat, onType = ::typeInstead, onDontSend = session::dontSend, onSendAgain = session::sendAgain)
            }
        }
        askOnResume = true
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        askOnResume = true
    }

    /** The unlock prompt can only be asked for by an activity that is showing, so a press is handled here. */
    override fun onResume() {
        super.onResume()
        if (awayToSettings) {
            awayToSettings = false
            if (AssistantRole.micAllowed(this)) session.listen() else session.micBlocked(still = true)
            return
        }
        if (!askOnResume || dismissing) return
        askOnResume = false
        unlockThen { micPressed() }
    }

    override fun onStop() {
        super.onStop()
        // Nothing listens once the sheet is gone: the microphone is only ever open in front of the owner.
        session.quiet()
        if (!isChangingConfigurations && !dismissing && !awayToSettings) finish()
    }

    override fun onDestroy() {
        super.onDestroy()
        session.close()
    }

    private fun micPressed() {
        val granted = ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
        if (granted) return session.micPressed()
        if (session.state.value.phase == VoicePhase.MicBlocked) return session.micBlocked()
        rationaleBefore = shouldShowRequestPermissionRationale(Manifest.permission.RECORD_AUDIO)
        askedAt = SystemClock.elapsedRealtime()
        AssistantRole.markMicAsked(this)
        askMic.launch(Manifest.permission.RECORD_AUDIO)
    }

    /** The chat this question is in, in the main app. Closing the sheet does not stop the answer. */
    private fun openChat(id: String) {
        val link = "hq://chat/$id"
        session.chatOpened()
        startActivity(Intent(this, example.hq.spike.ShellActivity::class.java).setData(Uri.parse(link)).putExtra(Links.EXTRA, link).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP))
        finish()
    }

    private fun openAppInfo() {
        awayToSettings = true
        try {
            startActivity(AssistantRole.appInfo(this))
        } catch (e: ActivityNotFoundException) {
            awayToSettings = false
        }
    }

    /** Type instead: the sheet closes and Chat opens with the keyboard up. The microphone is never opened for it. */
    private fun typeInstead() {
        startActivity(Intent(this, example.hq.spike.ShellActivity::class.java).setData(Uri.parse("hq://chat?compose=1")).putExtra(Links.EXTRA, "hq://chat?compose=1").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP))
        finish()
    }

    /** HQ's notes and tasks are never shown or heard over the lock screen: the owner unlocks first. */
    private fun unlockThen(then: () -> Unit) {
        val keyguard = getSystemService(KeyguardManager::class.java)
        if (!keyguard.isKeyguardLocked) {
            unlocked.value = true
            return then()
        }
        unlocked.value = false
        dismissing = true
        keyguard.requestDismissKeyguard(this, object : KeyguardManager.KeyguardDismissCallback() {
            override fun onDismissSucceeded() {
                dismissing = false
                Counters.get(this@VoiceActivity).bump(Counter.LockedUnlocked)
                unlocked.value = true
                then()
            }

            override fun onDismissCancelled() = gaveUp()

            override fun onDismissError() = gaveUp()
        })
    }

    private fun gaveUp() {
        Counters.get(this).bump(Counter.LockedCancelled)
        dismissing = false
        finish()
    }


    companion object {
        fun intent(context: Context): Intent = Intent(context, VoiceActivity::class.java)

    }
}

/** The sheet. While there is no model on the phone the shared download row and its asks sit where the round button does. */
@Composable
internal fun VoiceSheet(state: VoiceState, dl: DlState, item: Item, onOp: (Op) -> Unit, engine: () -> LargeDownload?, onMic: () -> Unit, onClose: () -> Unit, onSettings: () -> Unit = {}, onOpenChat: (String) -> Unit = {}, onType: () -> Unit = {}, onDontSend: () -> Unit = {}, onSendAgain: () -> Unit = {}) {
    val w = rememberWords()
    val needsModel = state.phase == VoicePhase.NeedsModel || state.phase == VoicePhase.Downloading
    Box(Modifier.fillMaxSize().background(Mts.Surface000.copy(alpha = SCRIM_ALPHA)).clickable(onClick = onClose), contentAlignment = Alignment.BottomCenter) {
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(topStart = SHEET_RADIUS.dp, topEnd = SHEET_RADIUS.dp)).background(Mts.Surface100)
                .clickable(enabled = false, onClick = {}).navigationBarsPadding().padding(SHEET_PADDING.dp),
            verticalArrangement = Arrangement.spacedBy(GAP.dp),
        ) {
            Text(VoiceWords.label(w, state), style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.semantics { heading() })
            VoiceWords.body(w, state.phase, dl)?.let { Text(it, style = HqType.BodySm, color = Mts.Ink) }
            if (state.heard.isNotBlank()) Text(state.heard, style = HqType.BodyBold, color = Mts.Ink)
            if (state.reply.isNotBlank()) {
                Text(state.reply, style = HqType.Body, color = Mts.Ink, modifier = Modifier.heightIn(max = REPLY_MAX_HEIGHT.dp).verticalScroll(rememberScrollState()))
            }
            val waiting = state.phase == VoicePhase.Thinking || state.phase == VoicePhase.LookingUp
            val late = androidx.compose.runtime.remember(waiting) { androidx.compose.runtime.mutableStateOf(false) }
            if (waiting) androidx.compose.runtime.LaunchedEffect(Unit) { kotlinx.coroutines.delay(CLOSE_HINT_MS); late.value = true }
            if (waiting && late.value) Text(w.s(R.string.voice_close_note), style = HqType.BodySm, color = Mts.InkMuted)
            if (state.phase == VoicePhase.Listening && state.cold && !state.slowLoad) Text(w.s(R.string.voice_cold_line), style = HqType.BodySm, color = Mts.InkMuted)
            if (state.reconnecting && state.phase in WORKING_PHASES) Text(w.s(R.string.voice_reconnecting_line), style = HqType.BodySm, color = Mts.InkMuted)
            if (state.needsOk) Text(w.s(R.string.voice_needs_ok_line), style = HqType.BodySm, color = Mts.Ink)
            if (state.afterClose && state.phase == VoicePhase.Idle && state.heard.isBlank()) Text(w.s(R.string.voice_next_open), style = HqType.BodySm, color = Mts.InkMuted)
            state.problem?.let { Problem(it) }
            if (state.phase == VoicePhase.MicBlocked) Text(w.s(R.string.assist_mic_taps), style = HqType.BodySm, color = Mts.InkMuted)
            if (needsModel) {
                DownloadRow(item, dl, onOp)
                DownloadDialogs(item, dl, engine)
                if (state.phase == VoicePhase.NeedsModel) TextButton(onClose, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.confirm_not_now), style = HqType.BodyBold, color = Mts.InkMuted) }
            } else {
                if (state.phase == VoicePhase.MicBlocked) {
                    SheetButton(w.s(R.string.assist_open_android), onClick = onSettings)
                    SheetButton(w.s(R.string.voice_ask_again), onClick = onMic)
                } else {
                    val chat = state.threadId?.takeIf { state.phase != VoicePhase.Idle || state.reply.isNotBlank() || state.afterClose }
                    val stack = androidx.compose.ui.platform.LocalDensity.current.fontScale >= STACK_FONT_SCALE
                    if (state.phase == VoicePhase.WaitingForHq) {
                        SheetButton(w.s(R.string.voice_dont_send), onClick = onDontSend)
                    } else if (chat != null && !stack) {
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(GAP.dp), verticalAlignment = Alignment.CenterVertically) {
                            MicButton(w, state.phase, onMic)
                            SheetButton(w.s(R.string.voice_open_chat), Modifier.weight(1f)) { onOpenChat(chat) }
                        }
                    } else {
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) { MicButton(w, state.phase, onMic) }
                        chat?.let { SheetButton(w.s(R.string.voice_open_chat)) { onOpenChat(it) } }
                    }
                    if (state.phase == VoicePhase.Idle && state.problem != null && state.heard.isNotBlank()) SheetButton(w.s(R.string.voice_send_again), onClick = onSendAgain)
                }
                val offerType = state.phase == VoicePhase.MicNeeded || state.phase == VoicePhase.MicBlocked || state.silent >= 2
                if (offerType) SheetButton(w.s(R.string.voice_type_instead), onClick = onType)
            }
        }
    }
}

/** A problem line: amber with an icon, so it is not told by colour alone. */
@Composable
private fun Problem(text: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Top) {
        androidx.compose.material3.Icon(example.hq.spike.ui.shell.HqIcons.Warning, null, Modifier.size(20.dp), tint = Mts.Amber)
        Text(text, style = HqType.BodySm, color = Mts.Ink)
    }
}

@Composable
private fun SheetButton(label: String, modifier: Modifier = Modifier.fillMaxWidth(), onClick: () -> Unit) {
    TextButton(onClick, modifier.heightIn(min = 48.dp)) { Text(label, style = HqType.BodyBold, color = Mts.Ink) }
}

@Composable
private fun MicButton(w: Words, phase: VoicePhase, onMic: () -> Unit) {
    val live = phase == VoicePhase.Listening
    val action = VoiceWords.action(w, phase)
    Box(
        Modifier.size(MIC_SIZE.dp).clip(CircleShape).background(if (live) Mts.BrandBlue else Mts.Surface200)
            .clickable(role = Role.Button, onClickLabel = action, onClick = onMic),
        contentAlignment = Alignment.Center,
    ) {
        Text(action, style = HqType.Caption, color = if (live) Mts.Surface000 else Mts.Ink)
    }
}

private const val STACK_FONT_SCALE = 1.5f
private val WORKING_PHASES = setOf(VoicePhase.Thinking, VoicePhase.LookingUp, VoicePhase.Answering)
private const val CLOSE_HINT_MS = 3_000L
private const val SCREEN_ON_MS = 30_000L
private const val SCRIM_ALPHA = 0.5f
private const val SHEET_RADIUS = 20
private const val SHEET_PADDING = 20
private const val GAP = 12
private const val REPLY_MAX_HEIGHT = 280
private const val MIC_SIZE = 72
