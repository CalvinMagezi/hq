package example.hq.spike.voice

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.ComponentName
import android.service.voice.VoiceInteractionService
import example.hq.spike.AlertPoster
import example.hq.spike.HubService
import example.hq.spike.R
import example.hq.spike.download.DlState
import example.hq.spike.download.Downloads
import example.hq.spike.ui.shell.Words

/**
 * One recogniser per process. Loading it takes seconds on a budget phone, so it is kept once loaded. The model's download is
 * not here: it is one job on the shared download engine ([Downloads]), which outlives any sheet.
 */
// ponytail: the recogniser is held until the process ends (about 70 MB); release on onTrimMemory if memory pressure shows up.
object VoiceEngines {
    const val READY_LINK = "hq://settings/speech"
    private const val READY_NOTICE_ID = 7_960
    @Volatile private var ears: Ears? = null
    @Volatile private var listening = false

    fun ears(context: Context): Ears = ears ?: synchronized(this) {
        ears ?: Ears(context.applicationContext).also { ears = it }
    }

    /** True while the speech model is on its way down, for the idle gate before an install. */
    fun downloading(): Boolean = Downloads.states.value[VoiceModels.ITEM_ID] is DlState.Downloading

    /** At the hub's start: once the model is checked, unpack it and say so, whether or not a sheet is open. */
    @Synchronized fun listen(context: Context) {
        if (listening) return
        listening = true
        val app = context.applicationContext
        Downloads.listen { item, s ->
            if (item.id == VoiceModels.ITEM_ID && s is DlState.Done) Thread({ if (VoiceModels(app).finish()) readyNotice(app) }, "speech-unpack").start()
        }
        Thread({ VoiceModels(app).finish() }, "speech-unpack-left").start()
    }

    /** Whether HQ is the phone's assistant: only then is "Hold Home to ask HQ." true. */
    fun isAssistant(context: Context): Boolean = VoiceInteractionService.isActiveService(context, ComponentName(context, AssistantService::class.java))

    private fun readyNotice(c: Context) {
        val w = Words.of(c)
        val b = Notification.Builder(c, AlertPoster.APP_UPDATES).setSmallIcon(HubService.statIcon(c)).setContentTitle(w.s(R.string.speech_ready_notice))
            .setContentIntent(AlertPoster.open(c, READY_LINK)).setAutoCancel(true).setOnlyAlertOnce(true)
        if (isAssistant(c)) b.setContentText(w.s(R.string.speech_ready_notice_body))
        c.getSystemService(NotificationManager::class.java)?.notify(READY_NOTICE_ID, b.build())
    }
}
