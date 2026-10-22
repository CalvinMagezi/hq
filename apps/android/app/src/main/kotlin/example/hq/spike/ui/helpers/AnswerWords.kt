package example.hq.spike.ui.helpers

/** What a notification says while an answer given from it is sent, and after: past tense, and what happens next. */
object AnswerWords {
    data class Say(val title: String, val body: String?)

    private const val NEEDS_OK = " needs your OK"
    private const val SOMEONE = "The helper"

    val sending = Say("Sending your answer…", null)
    val failed = Say("Couldn't send your answer", "HQ didn't reply. It may be restarting.")
    val answersOff = Say("Answers from notifications are off", "Open the app to answer.")
    val sentNotRecorded = Say("Answer sent", "HQ did not record it. Open the app to check.")

    /** The helper's name from the title its question carried ("Claude needs your OK"); null when the title is not one of those. */
    fun asker(title: String): String? = title.removeSuffix(NEEDS_OK).takeIf { it != title && it.isNotBlank() }

    /** [helper] is true for a question from a helper at a prompt; an approval of a task or an action has no one to name. */
    fun answered(helper: Boolean, approve: Boolean, name: String?, at: String): Say {
        val who = name ?: SOMEONE
        return when {
            helper && approve -> Say("Allowed. $who is working again.", "Sent at $at.")
            helper -> Say("Not now. $who is waiting.", "Sent at $at. You can tell it what to do instead.")
            approve -> Say("Allowed.", "Sent at $at.")
            else -> Say("Not now. Nothing was done.", "Sent at $at.")
        }
    }
}
