package example.hq.spike.ui.chat

import example.hq.spike.data.Offer

/** What the one slot above the composer holds. Follow-up chips will rank after the offer; they are not built yet. */
enum class SlotItem { None, Approval, Offer }

/** The three answers to an offer; [wire] is what `POST /api/offers/answer` takes. */
enum class OfferAnswer(val wire: String) { Yes("yes"), NotNow("not_now"), Later("later") }

/** The one line an answered offer leaves in the slot. Yes leaves none: the request is sent or put in the box. */
enum class OfferResult { Later, NotNow }

/** The composer's one button on the right. */
enum class SendKey { Voice, Send, Stop }

object ChatSlot {
    /** An approval wins over an offer, and an offer (or the line it left) shows only when no approval waits. Never two. */
    fun pick(hasApproval: Boolean, offered: Offer?, result: OfferResult?): SlotItem = when {
        hasApproval -> SlotItem.Approval
        offered != null || result != null -> SlotItem.Offer
        else -> SlotItem.None
    }

    /** Which of Yes, Not now and Later to draw. Later hides once the offer has come back from it, or on an HQ that has no Later. */
    fun answers(offer: Offer, laterSupported: Boolean): List<OfferAnswer> =
        if (laterSupported && !offer.laterUsed) listOf(OfferAnswer.Yes, OfferAnswer.NotNow, OfferAnswer.Later) else listOf(OfferAnswer.Yes, OfferAnswer.NotNow)

    /** An offer from before Later is scheduled with times; that stays for an HQ that cannot do Later. */
    fun schedulable(offer: Offer, laterSupported: Boolean): Boolean = !laterSupported && offer.kind.startsWith("next_")

    /** Voice when there is nothing to send, Send when there is, Stop while HQ works. */
    fun sendKey(running: Boolean, draft: String, hasAttachments: Boolean): SendKey = when {
        running -> SendKey.Stop
        draft.isBlank() && !hasAttachments -> SendKey.Voice
        else -> SendKey.Send
    }
}

/** The chats pinned on this phone: ids, most recently pinned first, kept in SharedPreferences only. */
object Pins {
    const val MAX = 6
    private const val SEPARATOR = ","

    sealed interface Change {
        data class Done(val ids: List<String>, val pinned: Boolean) : Change
        data object Full : Change
    }

    fun parse(stored: String?): List<String> = stored.orEmpty().split(SEPARATOR).filter { it.isNotBlank() }.distinct()

    fun encode(ids: List<String>): String = ids.joinToString(SEPARATOR)

    /** Pins or unpins [id]. It never prunes: HQ's list is one page, so a chat missing from it may still exist (see [forget]). */
    fun toggle(ids: List<String>, id: String): Change {
        if (id in ids) return Change.Done(ids - id, pinned = false)
        if (ids.size >= MAX) return Change.Full
        return Change.Done(listOf(id) + ids, pinned = true)
    }

    /** Drops the pin of a chat HQ says is gone (it answered 404 on open, or an archive was left to stand). */
    fun forget(ids: List<String>, id: String): List<String> = ids - id

    /** [rows] with pinned ones marked and first, in pin order; the rest keep their order. */
    fun apply(rows: List<ChatRow>, ids: List<String>): List<ChatRow> {
        val marked = rows.map { it.copy(pinned = it.id in ids) }
        return ids.mapNotNull { id -> marked.firstOrNull { it.id == id } } + marked.filterNot { it.pinned }
    }
}

/**
 * What an HQ's refusal of an offer answer means. A 404 is not a missing feature: it says that reply has no open offer (answered
 * on another device, or expired), so the offer is dropped and never put back. Only 405 and 501 say the route itself is absent.
 */
object OfferFailure {
    enum class Kind { RouteMissing, NoOpenOffer, LaterUsed, BadRequest, Transient }

    private const val BAD_REQUEST = 400
    private const val NOT_FOUND = 404
    private const val NOT_ALLOWED = 405
    private const val CONFLICT = 409
    private const val NOT_IMPLEMENTED = 501

    fun classify(status: Int): Kind = when (status) {
        NOT_ALLOWED, NOT_IMPLEMENTED -> Kind.RouteMissing
        NOT_FOUND -> Kind.NoOpenOffer
        CONFLICT -> Kind.LaterUsed
        BAD_REQUEST -> Kind.BadRequest
        else -> Kind.Transient
    }
}

object SlotLayout {
    /** From this font scale the buttons stack one under another instead of sharing a row. */
    const val STACK_FROM = 1.3f

    fun stacked(fontScale: Float) = fontScale >= STACK_FROM
}
