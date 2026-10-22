package example.hq.spike

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import example.hq.spike.data.HqClient.InboxItem
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.shell.Links
import example.hq.spike.ui.inbox.InboxCard
import example.hq.spike.ui.inbox.InboxWords
import org.json.JSONObject

/**
 * Turns the owner's pending items into Android notifications in the board's words: who needs what in a few words, a
 * body of one sentence, and buttons that are verbs. Allow asks for the phone to be unlocked first; Not now does not.
 * What is private is hidden on the lock screen.
 */
object AlertPoster {
    const val NEEDS = "needs"
    const val UPDATES = "updates"
    private const val MAX_BODY = 400
    private const val ID_BASE = 100
    private const val ID_SPAN = 100_000
    private const val SDK_AUTH_ACTION = 31
    private const val PREFS = "alerts"
    private const val ALLOW_ANSWERS = "allow_answers"

    @JvmStatic
    fun channels(): List<NotificationChannel> = listOf(
        NotificationChannel(NEEDS, "Needs you", NotificationManager.IMPORTANCE_HIGH).apply { description = "Questions that wait for your answer" },
        NotificationChannel(UPDATES, "Updates", NotificationManager.IMPORTANCE_DEFAULT).apply { description = "What HQ finished or noticed"; setSound(null, null) },
    )

    /** The Android notification number for an inbox id, the same each time so an answer can replace the question. */
    @JvmStatic
    fun notificationId(id: String): Int = ID_BASE + Math.abs(id.hashCode() % ID_SPAN)

    private fun answersAllowed(c: Context) = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(ALLOW_ANSWERS, true)

    fun itemOf(o: JSONObject): InboxItem {
        val meta = o.optJSONObject("metadata")
        val extra = meta?.optJSONObject("extra")
        val acts = o.optJSONArray("actions")
        return InboxItem(
            o.getString("id"), o.optString("kind", ""), o.optString("title", ""), o.optString("description", ""), o.optString("state", ""),
            o.optString("created_at", ""), meta?.optString("link")?.ifBlank { null }, extra?.optString("source_task")?.ifBlank { null }, extra?.optString("val_kind")?.ifBlank { null },
            (0 until (acts?.length() ?: 0)).map { acts!!.getString(it) },
        )
    }

    /** Oldest first so the newest ends on top; past [maxShown] one line stands for the rest. */
    @JvmStatic
    fun post(c: Context, items: Map<String, JSONObject>, fresh: List<String>, maxShown: Int) {
        val nm = c.getSystemService(NotificationManager::class.java)
        val shown = if (fresh.size > maxShown) fresh.subList(0, maxShown) else fresh
        for (id in shown.reversed()) {
            val card = InboxWords.card(itemOf(items.getValue(id)))
            nm.notify(notificationId(id), build(c, card))
        }
        if (fresh.size > shown.size) {
            nm.notify(ID_BASE - 1, simple(c, NEEDS, "${fresh.size - shown.size} more waiting for you", "Open AgentHQ to see them.", open(c)))
        }
    }

    /** An item answered somewhere else (a terminal, Telegram, the web) should not stay on the shade. */
    @JvmStatic
    fun cancelGone(c: Context, seen: List<String>, current: Set<String>) {
        val nm = c.getSystemService(NotificationManager::class.java)
        seen.filter { it !in current }.forEach { nm.cancel(notificationId(it)) }
    }

    /** Taps open what the notification is about: the helper for a helper's question, else the Inbox item; the Inbox when it is about nothing in particular. */
    internal fun open(c: Context, link: String = "hq://inbox"): PendingIntent {
        val intent = Intent(c, ShellActivity::class.java).setData(Uri.parse(link)).putExtra(Links.EXTRA, link).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        return PendingIntent.getActivity(c, link.hashCode(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    internal fun linkFor(card: InboxCard): String = when {
        card.kind == CardKind.Update -> "hq://inbox"
        card.sessionId != null -> "hq://helpers/${card.sessionId}"
        else -> "hq://inbox/${card.id}"
    }

    private fun simple(c: Context, channel: String, title: String, body: String, tap: PendingIntent): Notification =
        Notification.Builder(c, channel).setSmallIcon(HubService.statIcon(c)).setContentTitle(title).setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body)).setContentIntent(tap).setAutoCancel(true).build()

    private fun build(c: Context, card: InboxCard): Notification {
        val needs = card.kind != CardKind.Update
        val body = if (card.body.length > MAX_BODY) card.body.take(MAX_BODY) + "…" else card.body
        val b = Notification.Builder(c, if (needs) NEEDS else UPDATES).setSmallIcon(HubService.statIcon(c)).setContentTitle(card.title).setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body)).setContentIntent(open(c, linkFor(card))).setAutoCancel(true)
            .setCategory(if (needs) Notification.CATEGORY_REMINDER else Notification.CATEGORY_STATUS).setVisibility(Notification.VISIBILITY_PRIVATE)
        // The lock screen shows only that something is there, never what.
        val public = Notification.Builder(c, if (needs) NEEDS else UPDATES).setSmallIcon(HubService.statIcon(c))
            .setContentTitle(if (needs) "Something needs you" else "1 new update").setContentText(if (needs) "Unlock to see what." else "Unlock to read it.").build()
        b.setPublicVersion(public)
        if (needs) actions(c, card).forEach { b.addAction(it) }
        return b.build()
    }

    private fun actions(c: Context, card: InboxCard): List<Notification.Action> {
        val icon = Icon.createWithResource(c, HubService.statIcon(c))
        val out = mutableListOf<Notification.Action>()
        if (card.canAllow && answersAllowed(c) && Build.VERSION.SDK_INT >= SDK_AUTH_ACTION) {
            out += Notification.Action.Builder(icon, card.allowLabel, AnswerReceiver.intent(c, card, "approve")).setAuthenticationRequired(true).build()
            out += Notification.Action.Builder(icon, "Not now", AnswerReceiver.intent(c, card, "reject")).build()
        }
        out += Notification.Action.Builder(icon, if (card.kind == CardKind.HelperStuck) "Take a look" else "Open", open(c, linkFor(card))).build()
        return out
    }
}
