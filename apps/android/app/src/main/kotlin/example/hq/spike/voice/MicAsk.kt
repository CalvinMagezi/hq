package example.hq.spike.voice

import android.media.AudioManager

/** What Android's microphone dialog came to, read without storing anything about earlier refusals. */
enum class MicOutcome { Granted, Needed, Blocked, Dismissed }

object MicAsk {
    /** A dialog still on screen takes longer than this to answer; Android answering by itself does not. A guess to check on a phone. */
    const val DIALOG_MS = 300L

    /** Only Don't allow counts: a dismissed dialog leaves the rationale unchanged, and Android's own instant answer means it stopped asking. */
    fun outcome(granted: Boolean, rationaleBefore: Boolean, rationaleAfter: Boolean, tookMs: Long): MicOutcome = when {
        granted -> MicOutcome.Granted
        !rationaleBefore && rationaleAfter -> MicOutcome.Needed
        rationaleBefore && !rationaleAfter -> MicOutcome.Blocked
        rationaleBefore -> MicOutcome.Needed
        tookMs >= DIALOG_MS -> MicOutcome.Dismissed
        else -> MicOutcome.Blocked
    }

    /**
     * The microphone is held back: another app has it (not recording), or from Android 10 the system has silenced this
     * client (a call, another app with priority). Android 8 and 9 have only the recording state to go on.
     */
    fun blocked(recording: Boolean, sdk: Int, silenced: Boolean): Boolean = !recording || (sdk >= 29 && silenced)

    /** In a call the sheet does not try the microphone. */
    fun onCall(audioMode: Int): Boolean = audioMode == AudioManager.MODE_IN_CALL || audioMode == AudioManager.MODE_IN_COMMUNICATION

    /** Two silent tries in a row add the hint and Type instead; the microphone switch exists from Android 12 (API 31). */
    fun silenceLine(silentTries: Int, sdk: Int): SilenceLine = when {
        silentTries < 2 -> SilenceLine.Short
        sdk >= 31 -> SilenceLine.SwitchHint
        else -> SilenceLine.SpeakCloser
    }
}

enum class SilenceLine { Short, SwitchHint, SpeakCloser }
