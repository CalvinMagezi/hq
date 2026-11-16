package example.hq.spike.ui.chat

import android.content.Context
import example.hq.spike.R
import example.hq.spike.data.ChatEvent
import example.hq.spike.data.Offer

/** Client-side in-context asks and offers shown in the chat slot when no server offer or approval is waiting. */
enum class LocalOfferKind { Calendar, Messaging, Assistant, Repo }

data class LocalOffer(
    val kind: LocalOfferKind,
    val line: String,
    val yesLabel: String,
    val notNowLabel: String,
)

enum class CalendarPermOutcome { Granted, Denied, PermanentlyDenied }

/** Calendar permission ask: shown as a card in chat on the first calendar device action. */
object CalendarAsk {
    fun shouldShow(isCalendarRequest: Boolean, hasPermission: Boolean, askedBefore: Boolean): Boolean =
        isCalendarRequest && !hasPermission && !askedBefore

    fun isCalendarEvent(event: ChatEvent): Boolean =
        event is ChatEvent.DeviceAction && event.name.startsWith("calendar.")

    fun permissionOutcome(
        readGranted: Boolean,
        writeGranted: Boolean,
        rationaleRead: Boolean,
        rationaleWrite: Boolean,
    ): CalendarPermOutcome = when {
        readGranted && writeGranted -> CalendarPermOutcome.Granted
        !rationaleRead && !rationaleWrite -> CalendarPermOutcome.PermanentlyDenied
        else -> CalendarPermOutcome.Denied
    }
}

/** Messaging apps offer: shown after about three chats if no relay token is configured. */
object MessagingOffer {
    const val MIN_CHATS = 3

    fun shouldShow(chatCount: Int, hasRelayToken: Boolean, askedBefore: Boolean, replyDone: Boolean): Boolean =
        replyDone && chatCount >= MIN_CHATS && !hasRelayToken && !askedBefore
}

/** Phone assistant offer: shown after the first completed voice turn or dictation. */
object AssistantOffer {
    fun shouldShow(voiceTurnDone: Boolean, available: Boolean, held: Boolean, askedBefore: Boolean): Boolean =
        voiceTurnDone && available && !held && !askedBefore
}

/** GitHub and HQ Coder offer: shown when a GitHub repository URL appears in the user's message. */
object RepoOffer {
    private val FENCED_CODE = Regex("""```[\s\S]*?```""")
    private val INLINE_CODE = Regex("""`[^`\n]+`""")
    private val QUOTED_LINE = Regex("""(?m)^>.*$""")
    private val GITHUB_REPO_URL = Regex(
        """(?<![\w.-])(?:https?://)?(?:www\.)?github\.com/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)""",
        RegexOption.IGNORE_CASE,
    )
    private val RESERVED_SEGMENTS = setOf(
        "settings", "orgs", "marketplace", "notifications", "topics",
        "features", "pricing", "login", "about", "sponsors", "apps",
    )

    fun containsRepoUrl(text: String): Boolean {
        val plain = text
            .replace(FENCED_CODE, " ")
            .replace(INLINE_CODE, " ")
            .replace(QUOTED_LINE, " ")
        return GITHUB_REPO_URL.findAll(plain).any { m ->
            val owner = m.groupValues[1].lowercase()
            val repo = m.groupValues[2].lowercase()
            owner !in RESERVED_SEGMENTS && repo !in RESERVED_SEGMENTS
        }
    }

    fun shouldShow(userMessage: String, githubConnected: Boolean, askedBefore: Boolean): Boolean =
        containsRepoUrl(userMessage) && !githubConnected && !askedBefore
}

/** Pure wiring decisions shared between ChatViewModel and ShellActivity. */
object LaterAskDecider {
    fun blockedByOtherSlot(
        connected: Boolean,
        hasChatNeeds: Boolean,
        offered: Offer?,
        offerResult: OfferResult?,
        localOffer: LocalOffer?,
    ): Boolean = !connected || hasChatNeeds || offered != null || offerResult != null || localOffer != null

    fun shouldOfferMessagingOnTurnEnd(
        state: ChatUiState,
        hasRelayToken: Boolean,
        askedBefore: Boolean,
        threadMatches: Boolean,
        stopped: Boolean,
    ): Boolean =
        state.localOffer == null &&
            threadMatches &&
            !stopped &&
            MessagingOffer.shouldShow(
                chatCount = state.chats.size,
                hasRelayToken = hasRelayToken,
                askedBefore = askedBefore,
                replyDone = state.replyCompletedInSession,
            )

    fun shouldOfferRepoOnSubmit(
        fromComposer: Boolean,
        sendSucceeded: Boolean,
        text: String,
        githubConnected: Boolean,
        askedBefore: Boolean,
        hasLocalOffer: Boolean,
    ): Boolean =
        fromComposer &&
            sendSucceeded &&
            !hasLocalOffer &&
            RepoOffer.shouldShow(text, githubConnected, askedBefore)

    fun shouldOfferAssistantOnResume(
        hasLocalOffer: Boolean,
        voiceTurnDone: Boolean,
        available: Boolean,
        held: Boolean,
        askedBefore: Boolean,
    ): Boolean =
        !hasLocalOffer && AssistantOffer.shouldShow(voiceTurnDone, available, held, askedBefore)
}

/** Once-only memory for client-side in-context asks and offers, stored on this phone only. */
object LaterAsksPrefs {
    private const val FILE = "later_asks"
    private const val VOICE_TURN_DONE = "voice_turn_done"

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)
    private fun key(kind: LocalOfferKind) = "asked_${kind.name.lowercase()}"

    fun wasAsked(context: Context, kind: LocalOfferKind): Boolean = prefs(context).getBoolean(key(kind), false)

    fun markAsked(context: Context, kind: LocalOfferKind) {
        prefs(context).edit().putBoolean(key(kind), true).apply()
    }

    fun voiceTurnDone(context: Context): Boolean = prefs(context).getBoolean(VOICE_TURN_DONE, false)

    fun markVoiceTurnDone(context: Context) {
        prefs(context).edit().putBoolean(VOICE_TURN_DONE, true).apply()
    }

    fun buildOffer(context: Context, kind: LocalOfferKind): LocalOffer {
        val (lineId, yesId) = when (kind) {
            LocalOfferKind.Calendar -> R.string.chat_ask_calendar to R.string.chat_ask_calendar_allow
            LocalOfferKind.Messaging -> R.string.chat_offer_messaging to R.string.chat_offer_yes
            LocalOfferKind.Assistant -> R.string.chat_offer_assistant to R.string.chat_offer_yes
            LocalOfferKind.Repo -> R.string.chat_offer_repo to R.string.chat_offer_yes
        }
        return LocalOffer(
            kind = kind,
            line = context.getString(lineId),
            yesLabel = context.getString(yesId),
            notNowLabel = context.getString(R.string.chat_offer_not_now),
        )
    }
}
