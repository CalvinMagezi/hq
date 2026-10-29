package example.hq.spike

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.SystemClock
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.helpers.AnswerCheck
import example.hq.spike.ui.helpers.AnswerWords
import example.hq.spike.ui.helpers.HelperWords
import example.hq.spike.ui.inbox.InboxCard
import java.net.HttpURLConnection
import java.net.URL
import java.time.LocalTime
import java.time.format.DateTimeFormatter
import org.json.JSONArray
import org.json.JSONObject

/**
 * The answer given from a notification's button, sent to the HQ that asked (the one on this phone, or the owner's server
 * HQ for an id that carries [ServerHq.ID_PREFIX]). A helper at a prompt gets its key first (Enter to allow, Esc to
 * decline), then HQ records the answer. The notification is replaced by one that says what happened, and goes after a few
 * seconds; if HQ did not answer it says so and offers Try again. Nothing is queued.
 */
class AnswerReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getStringExtra(ID) ?: return
        val answer = intent.getStringExtra(ANSWER) ?: return
        val session = intent.getStringExtra(SESSION)
        val kind = intent.getStringExtra(KIND)
        val title = intent.getStringExtra(TITLE).orEmpty()
        val excerpt = intent.getStringExtra(EXCERPT).orEmpty().lines()
        // Cards posted before the owner turned answers off still carry their buttons.
        if (!AlertPoster.answersAllowed(context)) {
            context.getSystemService(NotificationManager::class.java).notify(AlertPoster.notificationId(id), say(context, AnswerWords.answersOff, null))
            return
        }
        // A question from the server HQ is answered there, under the id the server uses for it.
        val itemId = ServerHq.rawId(id)
        // A second tap before the card has changed would send the same key twice.
        if (!inFlight.add(id)) return
        val pending = goAsync()
        val deadline = SystemClock.elapsedRealtime() + DEADLINE_MS
        val nm = context.getSystemService(NotificationManager::class.java)
        val nid = AlertPoster.notificationId(id)
        AlertPoster.answeredNow(id)
        Thread {
            var keySent = false
            try {
                // The tap changes the asking card at once, with no buttons.
                nm.notify(nid, say(context, AnswerWords.sending, null))
                // Read here, off the main thread: a token is opened with the Keystore key.
                val target = Hq.of(context, id)
                if (target == null) {
                    nm.notify(nid, say(context, AnswerWords.Say("Not sent", "This phone is no longer signed in to your server HQ."), null))
                    return@Thread
                }
                // An answer given elsewhere stands: the server would take a second one over it. A check that cannot be made lets this one go.
                if (answeredElsewhere(itemId, target, deadline)) {
                    nm.notify(nid, say(context, AnswerWords.alreadyAnswered, null))
                    return@Thread
                }
                if (kind == CardKind.HelperAsks.name && session != null) {
                    val key = if (answer == "approve") "enter" else "esc"
                    // The key goes in only if the helper's live screen still shows the dialog this notification quoted.
                    val screen = JSONObject(get("/api/harness-sessions/$session/screen?lines=$CHECK_LINES", target, deadline))
                    val lines = screen.optJSONArray("lines")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
                    val check = HelperWords.checkAnswer(HelperWords.blocked(excerpt), lines, key, screen.optString("source") == "live")
                    if (check != AnswerCheck.Ok) {
                        nm.notify(nid, say(context, AnswerWords.Say("Not sent", HelperWords.checkWords(check)), null))
                        return@Thread
                    }
                    // Counted as sent once the request has gone out: an answer that then timed out may still have reached the helper, and a retry would press the key again.
                    post("/api/harness-sessions/$session/send", JSONObject().put("keys", JSONArray().put(key)), target, deadline) { keySent = true }
                }
                post("/api/notifications/$itemId/action", JSONObject().put("action", answer), target, deadline)
                val at = LocalTime.now().format(DateTimeFormatter.ofPattern("HH:mm"))
                val helper = kind == CardKind.HelperAsks.name && session != null
                nm.notify(nid, say(context, AnswerWords.answered(helper, answer == "approve", AnswerWords.asker(title), at), null))
            } catch (e: Exception) {
                // Once the key is in, a retry must not send it again: it would land on whatever the helper asks next.
                if (keySent) nm.notify(nid, say(context, AnswerWords.sentNotRecorded, null))
                else nm.notify(nid, say(context, AnswerWords.failed, intent))
            } finally {
                AlertPoster.answeredNow(id)
                inFlight.remove(id)
                pending.finish()
            }
        }.start()
    }

    private fun answeredElsewhere(id: String, target: Hq, deadline: Long): Boolean = try {
        val items = JSONObject(get("/api/notifications?state=history&limit=$HISTORY_LIMIT", target, deadline)).optJSONArray("notifications") ?: JSONArray()
        AnswerWords.answeredElsewhere(id, (0 until items.length()).mapNotNull { i -> items.optJSONObject(i)?.let { o -> o.optString("id") to o.optString("state") } })
    } catch (e: Exception) {
        false
    }

    /** The HQ an answer goes to: the one on this phone, over loopback, or the owner's server HQ with its device token. */
    private class Hq(val base: String, val token: String) {
        companion object {
            fun of(c: Context, id: String): Hq? = if (ServerHq.isServer(id)) {
                ServerHq.current(c)?.let { Hq(it.address, it.token) }
            } else {
                Hq("http://127.0.0.1:${HubService.PORT}", HubService.token(c))
            }
        }
    }

    private fun get(path: String, target: Hq, deadline: Long): String {
        val h = URL(target.base + path).openConnection() as HttpURLConnection
        try {
            h.connectTimeout = minOf(CONNECT_MS, left(deadline))
            h.readTimeout = left(deadline)
            h.instanceFollowRedirects = false
            h.setRequestProperty("Authorization", "Bearer " + target.token)
            h.setRequestProperty("X-HQ-Client", "android")
            if (h.responseCode !in 200..299) throw java.io.IOException("status ${h.responseCode}")
            return h.inputStream.bufferedReader().use { it.readText() }
        } finally {
            h.disconnect()
        }
    }

    private fun post(path: String, body: JSONObject, target: Hq, deadline: Long, afterWrite: () -> Unit = {}) {
        val h = URL(target.base + path).openConnection() as HttpURLConnection
        try {
            h.requestMethod = "POST"
            h.connectTimeout = minOf(CONNECT_MS, left(deadline))
            h.readTimeout = left(deadline)
            h.instanceFollowRedirects = false
            h.doOutput = true
            h.setRequestProperty("Authorization", "Bearer " + target.token)
            h.setRequestProperty("X-HQ-Client", "android")
            h.setRequestProperty("Content-Type", "application/json")
            h.outputStream.use { it.write(body.toString().toByteArray()) }
            afterWrite()
            if (h.responseCode !in 200..299) throw java.io.IOException("status ${h.responseCode}")
        } finally {
            h.disconnect()
        }
    }

    /** Milliseconds left of the deadline an answer has, from the tap (a connect can add up to two seconds more); none left means the answer failed. */
    private fun left(deadline: Long): Int = (deadline - SystemClock.elapsedRealtime()).toInt().also { if (it <= 0) throw java.io.IOException("no time left") }

    /** The notification that replaces the asking card: the words, and for a failure Try again (the same tap again) and Open. */
    private fun say(c: Context, words: AnswerWords.Say, retry: Intent?): Notification {
        val icon = android.graphics.drawable.Icon.createWithResource(c, HubService.statIcon(c))
        val b = Notification.Builder(c, AlertPoster.NEEDS).setSmallIcon(HubService.statIcon(c)).setContentTitle(words.title).setAutoCancel(true).setOnlyAlertOnce(true)
            .setContentIntent(AlertPoster.open(c))
        words.body?.let { b.setContentText(it) }
        if (words === AnswerWords.sending) b.setProgress(0, 0, true).setTimeoutAfter(SENDING_STUCK_MS)
        else if (retry == null) b.setTimeoutAfter(CLEAR_AFTER_MS)
        else {
            val again = Notification.Action.Builder(icon, "Try again", PendingIntent.getBroadcast(c, retry.getStringExtra(ID).hashCode(), Intent(retry), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
            // Allowing asks for the phone to be unlocked, so trying it again does too.
            if (retry.getStringExtra(ANSWER) == "approve" && Build.VERSION.SDK_INT >= SDK_AUTH_ACTION) again.setAuthenticationRequired(true)
            b.addAction(again.build())
            b.addAction(Notification.Action.Builder(icon, "Open", AlertPoster.open(c)).build())
        }
        return b.build()
    }

    companion object {
        private const val ID = "id"
        private const val ANSWER = "answer"
        private const val SESSION = "session"
        private const val KIND = "kind"
        private const val TITLE = "title"
        private const val EXCERPT = "excerpt"
        private const val CHECK_LINES = 81
        private const val HISTORY_LIMIT = 100
        private const val DEADLINE_MS = 10_000L
        private const val CONNECT_MS = 2_000
        private const val CLEAR_AFTER_MS = 6_000L
        private const val SENDING_STUCK_MS = 15_000L
        private const val SDK_AUTH_ACTION = 31
        private val inFlight: MutableSet<String> = java.util.concurrent.ConcurrentHashMap.newKeySet()

        fun intent(c: Context, card: InboxCard, answer: String): PendingIntent {
            val i = Intent(c, AnswerReceiver::class.java).putExtra(ID, card.id).putExtra(ANSWER, answer).putExtra(SESSION, card.sessionId)
                .putExtra(KIND, card.kind.name).putExtra(TITLE, card.title).putExtra(EXCERPT, card.excerpt.joinToString("\n"))
            return PendingIntent.getBroadcast(c, (card.id + answer).hashCode(), i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
    }
}
