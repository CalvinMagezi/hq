package example.hq.spike.ui.startup

import androidx.annotation.StringRes
import example.hq.spike.R
import example.hq.spike.data.HqStatus
import example.hq.spike.ui.shell.Words

/** The words of the "HQ is running" notice: they follow the state and the step, never the seconds, so it is posted only when they change. */
object RunningNotice {
    /** The step name for a 1 to 4 step. */
    @StringRes fun stepName(step: Int): Int = when (step.coerceIn(1, StartRules.STEPS)) {
        1 -> R.string.start_step_1
        2 -> R.string.start_step_2
        3 -> R.string.start_step_3
        else -> R.string.start_step_4
    }

    data class Text(val title: String, val body: String, val opensSheet: Boolean)

    fun of(w: Words, status: HqStatus, active: Boolean, serverAnswers: Boolean = false): Text = when (status) {
        is HqStatus.Starting -> Text(w.s(R.string.notice_starting), w.s(R.string.notice_starting_body, status.step, w.s(stepName(status.step))), false)
        is HqStatus.Restarting -> Text(w.s(R.string.notice_restarting), w.s(R.string.notice_restarting_body), false)
        is HqStatus.Stopped -> Text(w.s(R.string.notice_stopped), w.s(R.string.notice_stopped_body), true)
        else -> Text(
            w.s(R.string.notice_running),
            w.s(if (active) R.string.notice_active_body else if (serverAnswers) R.string.notice_standby_server_body else R.string.notice_standby_body),
            false,
        )
    }

    /** What the notice would say, as one key: posted again only when it changes. */
    fun key(w: Words, status: HqStatus, active: Boolean): String = of(w, status, active).let { it.title + "|" + it.body }
}
