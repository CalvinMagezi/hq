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
import example.hq.spike.ui.shell.Words
import example.hq.spike.ui.inbox.InboxCard
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
        val dialog = intent.getBooleanExtra(DIALOG, false)
        val title = intent.getStringExtra(TITLE).orEmpty()
        val excerpt = intent.getStringExtra(EXCERPT).orEmpty().lines()
        // A card posted before the owner turned details off, or before the phone locked, may still carry its buttons: nothing is sent for it.
        if (!AlertPoster.detailsNow(context)) {
            Alerts.refresh(context)
            return
        }
        // A question from the server HQ is answered there, under the id the server uses for it.
        val itemId = ServerHq.rawId(id)
        // A second tap before the card has changed would send the same key twice.
        if (!inFlight.add(id)) return
        val pending = goAsync()
        val w = Words.of(context)
        val deadline = SystemClock.elapsedRealtime() + DEADLINE_MS
        val nm = context.getSystemService(NotificationManager::class.java)
        val nid = AlertPoster.notificationId(id)
        AlertPoster.answeredNow(id)
        Thread {
            var keySent = false
            try {
                // The tap changes the asking card at once, with no buttons.
                nm.notify(nid, say(context, AnswerWords.sending(w), null))
                // Read here, off the main thread: a token is opened with the Keystore key.
                val target = Hq.of(context, id)
                if (target == null) {
                    nm.notify(nid, say(context, AnswerWords.Say("Not sent", "This phone is no longer signed in to your server HQ."), null))
                    return@Thread
                }
                if (dialog) {
                    // The server answers the dialog it read and refuses one that moved on, so nothing is read or typed here. A retry
                    // after a timeout is safe: the first answer claimed the dialog, so a second one is refused, never sent twice.
                    val choice = if (answer == "approve") "approve" else "decline"
                    val status = postStatus("/api/notifications/$itemId/answer", JSONObject().put("choice", choice), target, deadline)
                    val at = LocalTime.now().format(DateTimeFormatter.ofPattern("HH:mm"))
                    when (status) {
                        in 200..299 -> nm.notify(nid, say(context, AnswerWords.answered(w, true, answer == "approve", AnswerWords.asker(title), at), null))
                        409 -> nm.notify(nid, say(context, AnswerWords.movedOn(w), null))
                        else -> throw java.io.IOException("status $status")
                    }
                    return@Thread
                }
                // An answer given elsewhere stands: the server would take a second one over it. A check that cannot be made lets this one go.
                if (answeredElsewhere(itemId, target, deadline)) {
                    nm.notify(nid, say(context, AnswerWords.alreadyAnswered(w), null))
                    return@Thread
                }
                if (kind == CardKind.HelperAsks.name && session != null) {
                    val key = if (answer == "approve") "enter" else "esc"
                    // The key goes in only if the helper's live screen still shows the dialog this notification quoted.
                    val screen = JSONObject(get("/api/harness-sessions/$session/screen?lines=$CHECK_LINES", target, deadline))
                    val lines = screen.optJSONArray("lines")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
                    val check = HelperWords.checkAnswer(HelperWords.blocked(excerpt), lines, key, screen.optString("source") == "live")
                    if (check != AnswerCheck.Ok) {
                        nm.notify(nid, say(context, AnswerWords.Say("Not sent", HelperWords.checkWords(w, check)), null))
                        return@Thread
                    }
                    // Counted as sent once the request has gone out: an answer that then timed out may still have reached the helper, and a retry would press the key again.
                    post("/api/harness-sessions/$session/send", JSONObject().put("keys", JSONArray().put(key)), target, deadline) { keySent = true }
                }
                post("/api/notifications/$itemId/action", JSONObject().put("action", answer), target, deadline)
                val at = LocalTime.now().format(DateTimeFormatter.ofPattern("HH:mm"))
                val helper = kind == CardKind.HelperAsks.name && session != null
                nm.notify(nid, say(context, AnswerWords.answered(w, helper, answer == "approve", AnswerWords.asker(title), at), null))
            } catch (e: Exception) {
                // Once the key is in, a retry must not send it again: it would land on whatever the helper asks next.
                if (keySent) nm.notify(nid, say(context, AnswerWords.sentNotRecorded(w), null))
                else nm.notify(nid, say(context, failure(context, e), intent))
            } finally {
                AlertPoster.answeredNow(id)
                inFlight.remove(id)
                pending.finish()
            }
        }.start()
    }

    /** Couldn't send your answer, with the reason from the failure catalogue: no reply in time, or a server HQ that cannot be reached. */
    private fun failure(c: Context, e: Exception): AnswerWords.Say {
        val unreachable = e is java.net.ConnectException || e is java.net.UnknownHostException
        return AnswerWords.Say(c.getString(R.string.fail_send_answer_title), c.getString(if (unreachable) R.string.fail_send_answer_unreachable_body else R.string.fail_send_answer_no_reply_body))
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

    /** The phone's own hq gets the token only once it has proved it is hq (ServerProof); until then the answer fails as unreachable. */
    private fun requireProven(target: Hq) {
        if (!ServerProof.ensure(target.base, target.token)) throw java.net.ConnectException("hq has not shown it is the phone's HQ")
    }

    private fun get(path: String, target: Hq, deadline: Long): String {
        requireProven(target)
        val h = Loopback.open(URL(target.base + path))
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
        requireProven(target)
        val h = Loopback.open(URL(target.base + path))
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

    /** A POST whose status is the answer: 409 means the dialog was answered elsewhere or moved on, not a failure. */
    private fun postStatus(path: String, body: JSONObject, target: Hq, deadline: Long): Int {
        requireProven(target)
        val h = Loopback.open(URL(target.base + path))
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
            return h.responseCode
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
            .setContentIntent(AlertPoster.open(c)).setVisibility(Notification.VISIBILITY_PRIVATE).setPublicVersion(AlertPoster.publicVersion(c, AlertPoster.NEEDS))
        words.body?.let { b.setContentText(it) }
        if (words == AnswerWords.sending(Words.of(c))) b.setProgress(0, 0, true).setTimeoutAfter(SENDING_STUCK_MS)
        else if (retry == null) b.setTimeoutAfter(CLEAR_AFTER_MS)
        else {
            val again = Notification.Action.Builder(icon, c.getString(R.string.notice_action_try_again), PendingIntent.getBroadcast(c, retry.getStringExtra(ID).hashCode(), Intent(retry), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
            // Allowing asks for the phone to be unlocked, so trying it again does too.
            if (retry.getStringExtra(ANSWER) == "approve" && Build.VERSION.SDK_INT >= SDK_AUTH_ACTION) again.setAuthenticationRequired(true)
            b.addAction(again.build())
            b.addAction(Notification.Action.Builder(icon, c.getString(R.string.notice_action_open), AlertPoster.open(c)).build())
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
        private const val DIALOG = "dialog"
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
                .putExtra(KIND, card.kind.name).putExtra(TITLE, card.title).putExtra(EXCERPT, card.excerpt.joinToString("\n")).putExtra(DIALOG, card.dialog != null)
            return PendingIntent.getBroadcast(c, (card.id + answer).hashCode(), i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
    }
}
