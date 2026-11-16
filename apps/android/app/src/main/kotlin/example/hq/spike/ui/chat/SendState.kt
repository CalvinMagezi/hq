package example.hq.spike.ui.chat

enum class SendStatus { Sending, NotSent }

/**
 * Pure rules for per-message send state:
 * - Every user message with [UiMessage.notSent] keeps "Not sent" + "Try again".
 * - "Sending…" shows only under the last user message.
 */
object MessageSendState {
    fun statusFor(messages: List<UiMessage>, messageId: String): SendStatus? {
        val msg = messages.firstOrNull { it.id == messageId && it.speaker == Speaker.User } ?: return null
        if (msg.notSent) return SendStatus.NotSent
        val lastSent = messages.lastOrNull { it.speaker == Speaker.User }
        return if (msg.sending && lastSent?.id == messageId) SendStatus.Sending else null
    }

    /** Flips every in-flight local send to notSent when the socket drops. */
    fun failInFlight(messages: List<UiMessage>): List<UiMessage> =
        messages.map { if (it.speaker == Speaker.User && it.sending) it.copy(sending = false, notSent = true) else it }

    /** Keeps only local unsent/in-flight messages that HQ has not already saved under [UiMessage.clientId] or id. */
    fun unsavedLocals(localMessages: List<UiMessage>, savedFromServer: List<UiMessage>): List<UiMessage> {
        val savedIds = savedFromServer.mapTo(HashSet()) { it.id }
        val savedClientIds = savedFromServer.mapNotNullTo(HashSet()) { it.clientId }
        return localMessages.filter { m -> m.id.startsWith("local-") && m.id !in savedIds && m.id !in savedClientIds }
    }

    /** Transitions a notSent message to sending=true, notSent=false once; returns null if already sending or sent. */
    fun beginRetry(messages: List<UiMessage>, messageId: String): Pair<List<UiMessage>, UiMessage?> {
        val msg = messages.firstOrNull { it.id == messageId && it.speaker == Speaker.User }
        if (msg == null || !msg.notSent || msg.sending) return messages to null
        val updated = messages.map { if (it.id == messageId) it.copy(sending = true, notSent = false) else it }
        return updated to msg
    }

    /** When a rejected send had truncated a tail for edit/regenerate, reload the thread to restore it. */
    fun shouldReloadOnReject(replacedId: String?, threadId: String?): Boolean =
        replacedId != null && threadId != null
}
