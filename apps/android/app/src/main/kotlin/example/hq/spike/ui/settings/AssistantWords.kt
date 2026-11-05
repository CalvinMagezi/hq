package example.hq.spike.ui.settings

import example.hq.spike.R
import example.hq.spike.ui.shell.Words
import example.hq.spike.voice.MicFact

/** What the Phone assistant row and page say. Pure, so the order of the five statuses can be tested. */
object AssistantWords {
    /** First match wins: not on this phone, not set up, microphone off, no speech model, ready. */
    fun status(w: Words, available: Boolean, held: Boolean, mic: MicFact, modelReady: Boolean): String = w.s(
        when {
            !available -> R.string.assist_status_unavailable
            !held -> R.string.assist_status_not_set_up
            mic == MicFact.Off -> R.string.assist_status_mic_off
            !modelReady -> R.string.assist_status_model_missing
            else -> R.string.assist_status_ready
        },
    )

    /** Only the microphone can be a problem on the row; not being set up is a choice, so the row stays grey. */
    fun warn(available: Boolean, held: Boolean, mic: MicFact): Boolean = available && held && mic == MicFact.Off

    fun micLine(w: Words, mic: MicFact): String = w.s(
        when (mic) {
            MicFact.Allowed -> R.string.assist_mic_allowed
            MicFact.NotAsked -> R.string.assist_mic_not_asked
            MicFact.Off -> R.string.assist_mic_off
        },
    )

    /** Android 10 and later call the setting Digital assistant app; 8 and 9 call it Assist app. */
    fun firstStep(sdk: Int): Int = if (sdk >= 29) R.string.assist_step_digital else R.string.assist_step_assist
}
