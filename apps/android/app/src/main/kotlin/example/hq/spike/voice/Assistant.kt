package example.hq.spike.voice

import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.RemoteException
import android.service.voice.VoiceInteractionService
import android.service.voice.VoiceInteractionSession
import android.service.voice.VoiceInteractionSessionService
import android.speech.RecognitionService
import android.speech.SpeechRecognizer
import android.util.Log

/**
 * The service Android binds once the owner picks AgentHQ under Settings, Apps, Default apps, Digital assistant
 * app, and again after every boot once the phone is unlocked. Holding Home (or the assist gesture) opens a session.
 */
class AssistantService : VoiceInteractionService() {
    override fun onReady() {
        super.onReady()
        // Android would otherwise copy the text of whatever is on screen (and a screenshot) into this app on every
        // press. HQ does not use it, so it is never collected.
        setDisabledShowContext(VoiceInteractionSession.SHOW_WITH_ASSIST or VoiceInteractionSession.SHOW_WITH_SCREENSHOT)
        Log.i(TAG, "assistant ready")
    }

    override fun onLaunchVoiceAssistFromKeyguard() {
        VoiceActivity.open(this)
    }
}

class AssistantSessionService : VoiceInteractionSessionService() {
    override fun onNewSession(args: Bundle?): VoiceInteractionSession = AssistantSession(this)
}

/** Draws nothing itself: it opens the voice screen, which asks for the unlock first, and steps aside. */
class AssistantSession(context: Context) : VoiceInteractionSession(context) {
    override fun onShow(args: Bundle?, showFlags: Int) {
        super.onShow(args, showFlags)
        val intent = VoiceActivity.intent(context)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) startAssistantActivity(intent) else context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        hide()
    }
}

/**
 * Android lists an app as a possible assistant only when it names a recognition service. This one refuses every
 * request, so no other app can route its audio through AgentHQ by asking for the default recognizer.
 */
class NoRecognitionService : RecognitionService() {
    override fun onStartListening(recognizerIntent: Intent?, listener: Callback?) {
        try {
            listener?.error(SpeechRecognizer.ERROR_RECOGNIZER_BUSY)
        } catch (e: RemoteException) {
            Log.w(TAG, "recognition caller left: $e")
        }
    }

    override fun onCancel(listener: Callback?) = Unit

    override fun onStopListening(listener: Callback?) = Unit
}

private const val TAG = "HqVoice"
