package example.hq.spike

import android.content.Context
import example.hq.spike.ui.helpers.AnswerWords
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.inbox.InboxCard

/**
 * What a notification says, in two texts. [general] names who and what kind and never what: it is what is posted by
 * default, because history, watches and listeners keep exactly what was posted. [detail] adds the request's own words and
 * is posted only while the owner has asked for details, the phone is unlocked and has a secure lock.
 */
object NoticeWords {
    data class Words(val title: String, val body: String)

    private const val MAX_BODY = 400
    private const val UNKNOWN_HELPER = "A helper"

    fun general(c: Context, card: InboxCard): Words = when {
        card.kind == CardKind.Update -> Words(c.getString(R.string.notice_update_title), c.getString(R.string.notice_update_body))
        card.kind == CardKind.HelperStuck -> Words(c.getString(R.string.notice_helper_stuck_title), c.getString(R.string.notice_stuck_body))
        // A new sign-in is never a task needing an OK, and it is answered in the app, not from the shade.
        card.kind == CardKind.SignIn -> Words(c.getString(R.string.new_signin_title), c.getString(R.string.new_signin_advice))
        ServerHq.isServer(card.id) -> Words(c.getString(R.string.notice_server_ok_title), c.getString(R.string.notice_open_to_see_body))
        card.kind == CardKind.HelperAsks -> Words(c.getString(R.string.notice_helper_ok_title, helper(card)), c.getString(R.string.notice_open_to_see_body))
        else -> Words(c.getString(R.string.notice_task_ok_title), c.getString(R.string.notice_open_to_see_body))
    }

    /** The general title with the card's own body; a task's or an update's own title is never a notification title. */
    fun detail(c: Context, card: InboxCard): Words {
        val title = general(c, card).title
        val own = if (card.kind == CardKind.Update) listOf(card.title, card.body) else listOf(card.body)
        val body = own.filter { it.isNotBlank() }.joinToString("\n").let { if (it.length > MAX_BODY) it.take(MAX_BODY) + "…" else it }
        return Words(title, body.ifBlank { general(c, card).body })
    }

    /** "HQ answered": the words for an answer that is ready, never the reply itself. */
    fun answered(c: Context): Words = Words(c.getString(R.string.notice_answered_title), c.getString(R.string.notice_answered_body))

    /** The words a lock screen shows for owners who hide sensitive content. */
    fun publicWords(c: Context, needs: Boolean): Words =
        if (needs) Words(c.getString(R.string.notice_public_needs_title), c.getString(R.string.notice_public_needs_body))
        else Words(c.getString(R.string.notice_public_update_title), c.getString(R.string.notice_public_update_body))

    private fun helper(card: InboxCard): String = AnswerWords.asker(card.title) ?: UNKNOWN_HELPER

    /** A request HQ says takes something away: it shows its details like any other but offers Open only. */
    fun losing(card: InboxCard): Boolean = card.loses
}
