package example.hq.spike.voice

import android.Manifest
import android.app.KeyguardManager
import android.content.Context
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
 * The sheet "Hey HQ" and holding Home open. It hears one request, shows and speaks the reply, and listens again
 * on a tap. Audio is recognised on the phone by [VoiceSession]; only the final words go to HQ's chat.
 */
class VoiceActivity : ComponentActivity() {
    private lateinit var session: VoiceSession
    private val unlocked = androidx.compose.runtime.mutableStateOf(false)
    /** The unlock prompt is up; the sheet stays while it is, since the prompt itself hides the sheet. */
    private var dismissing = false
    /** A press (the first open or a new intent) waits for onResume to start. */
    private var askOnResume = false

    private val askMic = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) session.listen() else session.refuse(MIC_REFUSED)
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
                if (unlocked.value) VoiceSheet(state, onMic = ::micPressed, onClose = ::finish)
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
        if (!askOnResume || dismissing) return
        askOnResume = false
        unlockThen { micPressed() }
    }

    override fun onStop() {
        super.onStop()
        // Nothing listens or speaks once the sheet is gone: the microphone is only ever open in front of the owner.
        session.quiet()
        if (!isChangingConfigurations && !dismissing) finish()
    }

    override fun onDestroy() {
        super.onDestroy()
        session.close()
    }

    private fun micPressed() {
        val granted = ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
        if (granted) session.micPressed() else askMic.launch(Manifest.permission.RECORD_AUDIO)
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
                unlocked.value = true
                then()
            }

            override fun onDismissCancelled() = gaveUp()

            override fun onDismissError() = gaveUp()
        })
    }

    private fun gaveUp() {
        dismissing = false
        finish()
    }


    companion object {
        private const val MIC_REFUSED = "AgentHQ needs the microphone to hear you. Nothing you say leaves this phone as sound."

        fun intent(context: Context): Intent = Intent(context, VoiceActivity::class.java)

    }
}

@Composable
private fun VoiceSheet(state: VoiceState, onMic: () -> Unit, onClose: () -> Unit) {
    Box(Modifier.fillMaxSize().background(Mts.Surface000.copy(alpha = SCRIM_ALPHA)).clickable(onClick = onClose), contentAlignment = Alignment.BottomCenter) {
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(topStart = SHEET_RADIUS.dp, topEnd = SHEET_RADIUS.dp)).background(Mts.Surface100)
                .clickable(enabled = false, onClick = {}).navigationBarsPadding().padding(SHEET_PADDING.dp),
            verticalArrangement = Arrangement.spacedBy(GAP.dp),
        ) {
            val percent = state.progress?.let { " ${(it * PERCENT).toInt()}%" }.orEmpty()
            Text(state.phase.label + percent, style = HqType.Caption, color = Mts.InkMuted)
            state.notice?.let { Text(it, style = HqType.BodySm, color = Mts.Ink) }
            if (state.heard.isNotBlank()) Text(state.heard, style = HqType.BodyBold, color = Mts.Ink)
            if (state.reply.isNotBlank()) {
                Text(state.reply, style = HqType.Body, color = Mts.Ink, modifier = Modifier.heightIn(max = REPLY_MAX_HEIGHT.dp).verticalScroll(rememberScrollState()))
            }
            state.problem?.let { Text(it, style = HqType.BodySm, color = Mts.DangerText) }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center) { MicButton(state.phase, onMic) }
        }
    }
}

@Composable
private fun MicButton(phase: VoicePhase, onMic: () -> Unit) {
    val live = phase == VoicePhase.Listening
    Box(
        Modifier.size(MIC_SIZE.dp).clip(CircleShape).background(if (live) Mts.BrandBlue else Mts.Surface200)
            .clickable(role = Role.Button, onClickLabel = phase.action, onClick = onMic),
        contentAlignment = Alignment.Center,
    ) {
        Text(phase.action, style = HqType.Caption, color = if (live) Mts.Surface000 else Mts.Ink)
    }
}

private const val SCRIM_ALPHA = 0.5f
private const val PERCENT = 100
private const val SHEET_RADIUS = 20
private const val SHEET_PADDING = 20
private const val GAP = 12
private const val REPLY_MAX_HEIGHT = 280
private const val MIC_SIZE = 72
