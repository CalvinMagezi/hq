package example.hq.spike.ui.helpers

import example.hq.spike.R
import example.hq.spike.ui.shell.Words

/** What a notification says while an answer given from it is sent, and after: past tense, and what happens next. */
object AnswerWords {
    data class Say(val title: String, val body: String?)

    private const val NEEDS_OK = " needs your OK"

    fun sending(w: Words) = Say(w.s(R.string.helpers_answer_sending), null)
    fun sentNotRecorded(w: Words) = Say(w.s(R.string.helpers_answer_sent_title), w.s(R.string.helpers_answer_sent_body))
    fun alreadyAnswered(w: Words) = Say(w.s(R.string.helpers_answer_already_title), w.s(R.string.helpers_answer_already_body))
    fun movedOn(w: Words) = Say(w.s(R.string.helpers_answer_already_title), w.s(R.string.helpers_answer_moved_on_body))

    /** True only when HQ's answered items positively list this one as decided; anything else (not listed, cannot tell) lets the answer go. */
    fun answeredElsewhere(id: String, history: List<Pair<String, String>>): Boolean =
        history.any { (itemId, state) -> itemId == id && (state == "approved" || state == "dismissed") }

    /** The helper's name from the title its question carried ("Claude needs your OK"); null when the title is not one of those. */
    fun asker(title: String): String? = title.removeSuffix(NEEDS_OK).takeIf { it != title && it.isNotBlank() }

    /** [helper] is true for a question from a helper at a prompt; an approval of a task or an action has no one to name. */
    fun answered(w: Words, helper: Boolean, approve: Boolean, name: String?, at: String): Say {
        val who = name ?: w.s(R.string.helpers_the_helper)
        return when {
            helper && approve -> Say(w.s(R.string.helpers_answer_allowed_working, who), w.s(R.string.helpers_answer_sent_at, at))
            helper -> Say(w.s(R.string.helpers_answer_not_now_waiting, who), w.s(R.string.helpers_answer_sent_at_instead, at))
            approve -> Say(w.s(R.string.helpers_answer_allowed), w.s(R.string.helpers_answer_sent_at, at))
            else -> Say(w.s(R.string.helpers_answer_not_now_nothing), w.s(R.string.helpers_answer_sent_at, at))
        }
    }
}
