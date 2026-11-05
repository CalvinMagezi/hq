package example.hq.spike.voice

import example.hq.spike.R
import example.hq.spike.download.DlState
import example.hq.spike.ui.shell.Words

/** The words on the voice sheet, from the catalogue. */
object VoiceWords {
    fun label(w: Words, phase: VoicePhase): String = w.s(
        when (phase) {
            VoicePhase.NeedsModel -> R.string.speech_needed_title
            VoicePhase.Downloading -> R.string.speech_downloading_title
            VoicePhase.ModelReady -> R.string.speech_ready_title
            VoicePhase.Preparing -> R.string.voice_preparing
            VoicePhase.Listening -> R.string.voice_listening
            VoicePhase.Thinking -> R.string.voice_thinking
            VoicePhase.LookingUp -> R.string.voice_looking_up
            VoicePhase.Answering -> R.string.voice_answering
            VoicePhase.Idle -> R.string.voice_tap_to_ask
            VoicePhase.MicNeeded -> R.string.voice_mic_needed_label
            VoicePhase.MicBlocked -> R.string.voice_mic_off_label
            VoicePhase.Stopped -> R.string.voice_stopped
        },
    )

    fun action(w: Words, phase: VoicePhase): String = w.s(
        when (phase) {
            VoicePhase.Preparing, VoicePhase.NeedsModel, VoicePhase.Downloading -> R.string.voice_act_wait
            VoicePhase.Listening -> R.string.voice_act_done
            VoicePhase.Thinking, VoicePhase.LookingUp, VoicePhase.Answering -> R.string.voice_act_stop
            VoicePhase.Idle, VoicePhase.ModelReady -> R.string.voice_act_ask
            VoicePhase.MicNeeded -> R.string.voice_mic_allow_cd
            VoicePhase.MicBlocked -> R.string.voice_ask_again
            VoicePhase.Stopped -> R.string.voice_act_ask
        },
    )

    fun silence(w: Words, line: SilenceLine): String = w.s(
        when (line) {
            SilenceLine.Short -> R.string.voice_nothing_heard
            SilenceLine.SwitchHint -> R.string.voice_silence_switch
            SilenceLine.SpeakCloser -> R.string.voice_silence_closer
        },
    )

    /** The line under the label: what the model is for, where the download stands, or that a tap asks. */
    fun body(w: Words, phase: VoicePhase, dl: DlState): String? = when (phase) {
        VoicePhase.NeedsModel -> w.s(R.string.speech_needed_body)
        VoicePhase.Downloading -> w.s(if (dl == DlState.WaitingWifi) R.string.speech_waiting_body else R.string.speech_downloading_body)
        VoicePhase.ModelReady -> w.s(R.string.speech_ready_body)
        VoicePhase.MicNeeded -> w.s(R.string.voice_mic_needed_body)
        VoicePhase.MicBlocked -> w.s(R.string.voice_mic_blocked_body)
        else -> null
    }
}
