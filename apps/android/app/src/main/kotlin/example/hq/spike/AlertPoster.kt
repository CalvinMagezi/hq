package example.hq.spike

import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import android.os.SystemClock
import example.hq.spike.data.HqClient.InboxItem
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.shell.Links
import example.hq.spike.ui.inbox.InboxCard
import example.hq.spike.ui.inbox.InboxWords
import org.json.JSONObject

/**
 * Turns the owner's pending items into Android notifications. By default a notice says who and what kind
 * ([NoticeWords.general]) with Open as its only button. The details, and Decline and Allow once, are posted only while the
 * owner has switched details on, the phone has a secure lock and it is unlocked ([detailsNow]); the hub posts the general
 * words again when the phone locks. Every notice also carries a public version for owners who hide sensitive content.
 */
object AlertPoster {
    const val NEEDS = "needs"
    const val UPDATES = "updates"
    const val APP_UPDATES = "app_updates"
    const val DOWNLOADS = "downloads"
    const val HUB = "hub"
    private const val ID_BASE = 100
    private const val ID_SPAN = 100_000
    private const val SDK_AUTH_ACTION = 31
    private const val PREFS = "alerts"
    private const val SHOW_DETAILS = "show_details"
    private const val OLD_ALLOW_ANSWERS = "allow_answers"
    private const val UPGRADE_CHECKED = "details_migrated"
    private const val UPGRADER_NOTE = "upgrader_note"
    private const val SEEN = "seen"
    private const val RESULT_KEEP_MS = 15_000L
    private val answeredAt = java.util.concurrent.ConcurrentHashMap<String, Long>()
    @Volatile private var appContext: Context? = null

    /** The five channels, made once at the hub's start; names come from the string catalogue. */
    @JvmStatic
    fun channels(c: Context): List<NotificationChannel> {
        appContext = c.applicationContext
        fun channel(id: String, name: Int, description: Int, importance: Int, silent: Boolean = false) =
            NotificationChannel(id, c.getString(name), importance).apply { this.description = c.getString(description); if (silent) setSound(null, null) }
        return listOf(
            channel(NEEDS, R.string.channel_needs_name, R.string.channel_needs_description, NotificationManager.IMPORTANCE_HIGH),
            channel(UPDATES, R.string.channel_updates_name, R.string.channel_updates_description, NotificationManager.IMPORTANCE_DEFAULT, silent = true),
            channel(APP_UPDATES, R.string.channel_app_updates_name, R.string.channel_app_updates_description, NotificationManager.IMPORTANCE_LOW),
            channel(DOWNLOADS, R.string.channel_downloads_name, R.string.channel_downloads_description, NotificationManager.IMPORTANCE_LOW),
            channel(HUB, R.string.channel_hub_name, R.string.channel_hub_description, NotificationManager.IMPORTANCE_MIN),
        )
    }

    /** For a caller with no context at hand: the channels once any caller with one has asked for them, else none. */
    @JvmStatic
    fun channels(): List<NotificationChannel> = appContext?.let { channels(it) } ?: emptyList()

    /** The Android notification number for an inbox id, the same each time so an answer can replace the question. */
    @JvmStatic
    fun notificationId(id: String): Int = ID_BASE + Math.abs(id.hashCode() % ID_SPAN)

    /** The owner's choice to see details in notifications. Off unless they turned it on: the old "answers" switch is not carried over. */
    @JvmStatic
    fun showDetails(c: Context): Boolean = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(SHOW_DETAILS, false)

    /** Stores the choice and re-posts every shown notice at once, so turning details off takes them off the shade. */
    @JvmStatic
    fun setShowDetails(c: Context, on: Boolean) {
        c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(SHOW_DETAILS, on).apply()
        Alerts.refresh(c)
    }

    /** True when the phone has a PIN, pattern or password; without one a swipe opens it, so details are never posted. */
    @JvmStatic
    fun deviceSecure(c: Context): Boolean = c.getSystemService(KeyguardManager::class.java)?.isDeviceSecure == true

    /** Rule 5: the switch is on, the phone has a secure lock and it is unlocked at this moment. */
    @JvmStatic
    fun detailsNow(c: Context): Boolean {
        val km = c.getSystemService(KeyguardManager::class.java) ?: return false
        return showDetails(c) && km.isDeviceSecure && !km.isKeyguardLocked
    }

    /**
     * Run once at the hub's start. An owner who already had a seen list is upgrading: details start off, and they get a
     * one-time note on the Notifications page. The old answers switch is dropped.
     */
    @JvmStatic
    fun migrate(c: Context) {
        val p = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (p.getBoolean(UPGRADE_CHECKED, false)) return
        p.edit().putBoolean(UPGRADE_CHECKED, true).remove(OLD_ALLOW_ANSWERS).putBoolean(UPGRADER_NOTE, p.contains(SEEN)).apply()
    }

    /** True once for an upgrader: read by the Notifications page, which shows its card and then lets the note go. */
    internal fun upgraderNote(c: Context): Boolean {
        migrate(c)
        return c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(UPGRADER_NOTE, false)
    }

    internal fun clearUpgraderNote(c: Context) {
        c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(UPGRADER_NOTE, false).apply()
    }

    /** True for an item that waits on the owner; an update is only news and does not count toward "Needs you". */
    @JvmStatic
    fun needsYou(o: JSONObject): Boolean = example.hq.spike.ui.inbox.InboxWords.card(itemOf(o)).kind != CardKind.Update

    fun itemOf(o: JSONObject): InboxItem {
        val meta = o.optJSONObject("metadata")
        val extra = meta?.optJSONObject("extra")
        val acts = o.optJSONArray("actions")
        return InboxItem(
            o.getString("id"), o.optString("kind", ""), o.optString("title", ""), o.optString("description", ""), o.optString("state", ""),
            o.optString("created_at", ""), meta?.optString("link")?.ifBlank { null }, extra?.optString("source_task")?.ifBlank { null }, extra?.optString("val_kind")?.ifBlank { null },
            (0 until (acts?.length() ?: 0)).map { acts!!.getString(it) }, example.hq.spike.data.alertDialog(meta),
        )
    }

    /** Oldest first so the newest ends on top; past [maxShown] one line stands for the rest. Returns the ids that got their own notice. */
    @JvmStatic
    fun post(c: Context, items: Map<String, JSONObject>, fresh: List<String>, maxShown: Int): List<String> {
        val nm = c.getSystemService(NotificationManager::class.java)
        val shown = if (fresh.size > maxShown) fresh.subList(0, maxShown) else fresh
        val details = detailsNow(c)
        for (id in shown.reversed()) {
            val card = InboxWords.card(itemOf(items.getValue(id)))
            nm.notify(notificationId(id), build(c, card, details, quiet = false))
        }
        if (fresh.size > shown.size) {
            val more = fresh.size - shown.size
            nm.notify(ID_BASE - 1, simple(c, NEEDS, c.getString(R.string.needs_folded, more.toString()), c.getString(R.string.needs_folded_body), open(c)))
        }
        return shown
    }

    /**
     * Posts [items] again over what is on the shade, without sound or pop-up: general words when the phone is locking
     * ([forceGeneral], since the keyguard may not read as locked yet) or details are off, details when rule 5 holds.
     * A notice the owner already swiped away, or one that now shows an answer's result, is left alone.
     */
    @JvmStatic
    fun repost(c: Context, items: Map<String, JSONObject>, forceGeneral: Boolean) {
        val nm = c.getSystemService(NotificationManager::class.java)
        val active = nm.activeNotifications.map { it.id }.toSet()
        val now = SystemClock.elapsedRealtime()
        val details = !forceGeneral && detailsNow(c)
        for ((id, item) in items) {
            val nid = notificationId(id)
            if (nid !in active || now - (answeredAt[id] ?: Long.MIN_VALUE / 2) <= RESULT_KEEP_MS) continue
            nm.notify(nid, build(c, InboxWords.card(itemOf(item)), details, quiet = true))
        }
    }

    /** An item answered somewhere else (a terminal, Telegram, the web) should not stay on the shade. */
    @JvmStatic
    fun cancelGone(c: Context, seen: List<String>, current: Set<String>) {
        val nm = c.getSystemService(NotificationManager::class.java)
        val now = SystemClock.elapsedRealtime()
        // An item answered from its notification is no longer pending, but the card that says what happened must stay its few seconds.
        seen.filter { it !in current && now - (answeredAt[it] ?: Long.MIN_VALUE / 2) > RESULT_KEEP_MS }.forEach { nm.cancel(notificationId(it)) }
    }

    /** Records that [id] was just answered from its notification, so the sweep leaves the card that replaced the question alone. */
    internal fun answeredNow(id: String) {
        answeredAt[id] = SystemClock.elapsedRealtime()
    }

    /** Taps open what the notification is about: the helper for a helper's question, else the Inbox item; the Inbox when it is about nothing in particular. */
    internal fun open(c: Context, link: String = "hq://inbox"): PendingIntent {
        val intent = Intent(c, ShellActivity::class.java).setData(Uri.parse(link)).putExtra(Links.EXTRA, link).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        return PendingIntent.getActivity(c, link.hashCode(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    /** What a tap on [card] opens: for a question from the server HQ, that HQ's own web app, since this phone's pages do not know its helpers. */
    internal fun tapFor(c: Context, card: InboxCard): PendingIntent = if (ServerHq.isServer(card.id)) openServer(c) else open(c, linkFor(card))

    /** The server HQ's notifications page in the browser, or this app's Inbox when no server is kept. */
    internal fun openServer(c: Context): PendingIntent {
        val address = ServerHq.address(c) ?: return open(c)
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse("$address/notifications")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        return PendingIntent.getActivity(c, address.hashCode(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    /** One notice when the server HQ signs this phone out (Settings, Devices there, or Sign out everywhere): its questions stop arriving here. It names no address. */
    @JvmStatic
    fun serverSignedOut(c: Context) {
        c.getSystemService(NotificationManager::class.java).notify(
            ID_BASE - 2,
            simple(c, NEEDS, c.getString(R.string.notice_server_signed_out_title), c.getString(R.string.notice_server_signed_out_body), open(c)),
        )
    }

    internal fun linkFor(card: InboxCard): String = when {
        card.kind == CardKind.Update -> "hq://inbox"
        card.sessionId != null -> "hq://helpers/${card.sessionId}"
        else -> "hq://inbox/${card.id}"
    }

    /** The words a lock screen shows when the owner hides sensitive content; every notice carries them, whatever else it says. */
    internal fun publicVersion(c: Context, channel: String): Notification {
        val words = NoticeWords.publicWords(c, channel == NEEDS)
        return Notification.Builder(c, channel).setSmallIcon(HubService.statIcon(c)).setContentTitle(words.title).setContentText(words.body).build()
    }

    private fun simple(c: Context, channel: String, title: String, body: String, tap: PendingIntent): Notification =
        Notification.Builder(c, channel).setSmallIcon(HubService.statIcon(c)).setContentTitle(title).setContentText(body)
            .setStyle(Notification.BigTextStyle().bigText(body)).setContentIntent(tap).setAutoCancel(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE).setPublicVersion(publicVersion(c, channel)).build()

    internal fun build(c: Context, card: InboxCard, details: Boolean, quiet: Boolean): Notification {
        val needs = card.kind != CardKind.Update
        val words = if (details) NoticeWords.detail(c, card) else NoticeWords.general(c, card)
        val channel = if (needs) NEEDS else UPDATES
        val b = Notification.Builder(c, channel).setSmallIcon(HubService.statIcon(c)).setContentTitle(words.title).setContentText(words.body)
            .setStyle(Notification.BigTextStyle().bigText(words.body)).setContentIntent(tapFor(c, card)).setAutoCancel(true).setOnlyAlertOnce(quiet)
            .setCategory(if (needs) Notification.CATEGORY_REMINDER else Notification.CATEGORY_STATUS).setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(publicVersion(c, channel))
        if (needs) actions(c, card, details).forEach { b.addAction(it) }
        return b.build()
    }

    /** Which buttons a notice gets: Decline, Allow once, Open in that order only under rule 5 on Android 12+ and never for a losing request; else Open alone. */
    private fun actions(c: Context, card: InboxCard, details: Boolean): List<Notification.Action> {
        val icon = Icon.createWithResource(c, HubService.statIcon(c))
        val out = mutableListOf<Notification.Action>()
        val buttons = details && Build.VERSION.SDK_INT >= SDK_AUTH_ACTION && !NoticeWords.losing(card)
        if (buttons) {
            val answerable = card.dialog != null || card.canAllow
            // The server answers the dialog it read: Allow once only when it named a yes-once answer, Decline always.
            if (answerable) out += Notification.Action.Builder(icon, c.getString(R.string.notice_action_decline), AnswerReceiver.intent(c, card, "reject")).build()
            if (card.canAllow) out += Notification.Action.Builder(icon, c.getString(R.string.notice_action_allow_once), AnswerReceiver.intent(c, card, "approve")).setAuthenticationRequired(true).build()
        }
        out += Notification.Action.Builder(icon, c.getString(R.string.notice_action_open), tapFor(c, card)).build()
        return out
    }
}
