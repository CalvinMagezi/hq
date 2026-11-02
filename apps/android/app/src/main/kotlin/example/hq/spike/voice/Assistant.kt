package example.hq.spike.voice

import android.app.KeyguardManager
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
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

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
}

class AssistantSessionService : VoiceInteractionSessionService() {
    override fun onNewSession(args: Bundle?): VoiceInteractionSession = AssistantSession(this)
}

/** Draws nothing itself: it opens the voice screen, which asks for the unlock first, and steps aside. */
class AssistantSession(context: Context) : VoiceInteractionSession(context) {
    override fun onShow(args: Bundle?, showFlags: Int) {
        super.onShow(args, showFlags)
        // Locked: nothing opens and the microphone stays off. The owner unlocks and asks again.
        if (context.getSystemService(KeyguardManager::class.java).isKeyguardLocked) return hide()
        val intent = VoiceActivity.intent(context)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) startAssistantActivity(intent) else context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        hide()
    }
}

/**
 * An assistant must name a recognition service, and some phones then make it the default speech recogniser. So
 * this one really recognises, with the same on-device model, and other apps' dictation keeps working and stays on
 * the phone. Android lets only callers that hold the microphone permission in (RecognitionService checks it); they
 * get their own words back and nothing else. While the voice sheet holds the microphone they get "busy".
 */
class OnDeviceRecognitionService : RecognitionService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var job: Job? = null
    @Volatile private var hearing = AtomicBoolean(true)

    override fun onStartListening(recognizerIntent: Intent?, listener: Callback?) {
        val caller = listener ?: return
        job?.cancel()
        if (!VoiceModels(this).ready()) return tell(caller) { it.error(NO_MODELS) }
        val ears = VoiceEngines.ears(applicationContext)
        val stop = AtomicBoolean(false).also { hearing = it }
        job = scope.launch {
            try {
                ears.prepare()
                val text = ears.hear(
                    stop,
                    wait = false,
                    onRecording = { tell(caller) { it.readyForSpeech(Bundle()) } },
                    onPartial = { partial -> tell(caller) { it.partialResults(results(partial)) } },
                )
                tell(caller) { if (text.isBlank()) it.error(SpeechRecognizer.ERROR_NO_MATCH) else it.results(results(text)) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Ears.Busy) {
                tell(caller) { it.error(SpeechRecognizer.ERROR_RECOGNIZER_BUSY) }
            } catch (e: Exception) {
                Log.w(TAG, "dictation for another app failed", e)
                tell(caller) { it.error(SpeechRecognizer.ERROR_CLIENT) }
            }
        }
    }

    /** Ends this service's own request only; the voice sheet's listening is never touched from here. */
    override fun onStopListening(listener: Callback?) = hearing.set(true)

    override fun onCancel(listener: Callback?) {
        job?.cancel()
        hearing.set(true)
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun results(text: String) = Bundle().apply { putStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION, arrayListOf(text)) }

    /** The caller may have gone away; that ends its request and nothing else. */
    private fun tell(caller: Callback, call: (Callback) -> Unit) {
        try {
            call(caller)
        } catch (e: RemoteException) {
            Log.w(TAG, "recognition caller left: $e")
        }
    }

    private companion object {
        /** Before the owner has downloaded the models there is nothing to recognise with. */
        val NO_MODELS = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE else SpeechRecognizer.ERROR_CLIENT
    }
}

private const val TAG = "HqVoice"
