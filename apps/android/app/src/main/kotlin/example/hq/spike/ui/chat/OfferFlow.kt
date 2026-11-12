package example.hq.spike.ui.chat

import example.hq.spike.data.Offer

/** The two lines the offer and pill flows may need, from the catalogue. */
data class FlowTexts(val retry: String, val gone: String)

/**
 * What happens to the chat's state around an answer to an offer. Pure, so each path can be tested without a phone: an answer
 * collapses the offer at once, a failure puts it back (or drops it for good), and a reload never brings an answered one back.
 */
object OfferFlow {
    private fun resultOf(answer: OfferAnswer) = when (answer) {
        OfferAnswer.Later -> OfferResult.Later
        OfferAnswer.NotNow -> OfferResult.NotNow
        OfferAnswer.Yes -> null
    }

    private fun withOffer(messages: List<UiMessage>, id: String, offer: Offer?) = messages.map { if (it.id == id) it.copy(offer = offer) else it }

    /** The person answered: the offer leaves its reply, the id is remembered, and Later or Not now leave their one line. */
    fun answered(s: ChatUiState, id: String, answer: OfferAnswer): ChatUiState =
        s.copy(messages = withOffer(s.messages, id, null), offerResult = resultOf(answer), answeredOffers = s.answeredOffers + id)

    /** True when this answer should be sent: the offer is on screen and was not answered already (a double tap sends once). */
    fun canAnswer(s: ChatUiState, id: String): Boolean = id !in s.answeredOffers && s.messages.firstOrNull { it.id == id }?.offer != null

    /** Whether Yes still goes ahead after HQ refused to record it: only when the route is absent. */
    fun yesProceeds(status: Int?): Boolean = status == null || OfferFailure.classify(status) == OfferFailure.Kind.RouteMissing

    /**
     * HQ refused or could not be reached ([status] 0 for no answer). A 404 drops the offer for good with a quiet line. A missing
     * Later route hides Later. Anything else puts the offer back, so the result line never says something that did not happen.
     */
    fun failed(s: ChatUiState, id: String, offer: Offer, answer: OfferAnswer, status: Int, t: FlowTexts): ChatUiState {
        fun restore(back: Offer, notice: Notice?, laterSupported: Boolean = s.laterSupported) = s.copy(
            messages = withOffer(s.messages, id, back), offerResult = null, answeredOffers = s.answeredOffers - id,
            laterSupported = laterSupported, notice = notice ?: s.notice,
        )
        return when (OfferFailure.classify(status)) {
            OfferFailure.Kind.NoOpenOffer -> s.copy(offerResult = null, notice = Notice(t.gone))
            OfferFailure.Kind.RouteMissing -> if (answer == OfferAnswer.Later) restore(offer, null, laterSupported = false) else s
            OfferFailure.Kind.LaterUsed -> if (answer == OfferAnswer.Later) restore(offer.copy(laterUsed = true), null) else s
            OfferFailure.Kind.BadRequest, OfferFailure.Kind.Transient -> restore(offer, Notice(t.retry))
        }
    }

    /** [messages] with the offer taken off every reply the person already answered, so a reload cannot bring one back. */
    fun strip(messages: List<UiMessage>, answered: Set<String>): List<UiMessage> =
        if (answered.isEmpty()) messages else messages.map { if (it.offer != null && it.id in answered) it.copy(offer = null) else it }
}

/** Fewer like this and Never show this on a pill. Pure, like [OfferFlow]. */
object PillFlow {
    /** The pill leaves at once; nothing is announced until HQ has answered. */
    fun removed(s: ChatUiState, capability: String): ChatUiState {
        val left = Suggestions.without(s.suggestions, capability)
        // The last pill HQ chose is gone: show none until the next refresh, not the three fixed ones with no menu.
        return s.copy(suggestions = left, pillsCleared = s.pillsCleared || (!s.suggestions.isNullOrEmpty() && left.isNullOrEmpty()))
    }

    fun confirmed(s: ChatUiState, text: String): ChatUiState = s.copy(notice = Notice(text))

    /** [before] is the state when the pill was removed. A missing route hides the menu without touching any notice; other failures say try again. */
    fun failed(s: ChatUiState, before: ChatUiState, status: Int, t: FlowTexts): ChatUiState {
        val restored = s.copy(suggestions = before.suggestions, pillsCleared = before.pillsCleared)
        return if (OfferFailure.classify(status) == OfferFailure.Kind.RouteMissing || status == NOT_FOUND) restored.copy(pillChoices = false) else restored.copy(notice = Notice(t.retry))
    }

    private const val NOT_FOUND = 404
}
