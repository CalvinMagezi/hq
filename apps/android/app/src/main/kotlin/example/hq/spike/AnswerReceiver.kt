package example.hq.spike

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
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
 * The answer given from a notification's button. A helper at a prompt gets its key first (Enter to allow, Esc to
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
        val pending = goAsync()
        val deadline = SystemClock.elapsedRealtime() + DEADLINE_MS
        val nm = context.getSystemService(NotificationManager::class.java)
        val nid = AlertPoster.notificationId(id)
        // The tap changes the asking card at once, with no buttons, so a second tap cannot send twice.
        nm.notify(nid, say(context, AnswerWords.sending, null))
        Thread {
            var keySent = false
            try {
                if (kind == CardKind.HelperAsks.name && session != null) {
                    val key = if (answer == "approve") "enter" else "esc"
                    // The key goes in only if the helper's live screen still shows the dialog this notification quoted.
                    val screen = JSONObject(get("/api/harness-sessions/$session/screen?lines=$CHECK_LINES", context, deadline))
                    val lines = screen.optJSONArray("lines")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
                    val check = HelperWords.checkAnswer(HelperWords.blocked(excerpt), lines, key, screen.optString("source") == "live")
                    if (check != AnswerCheck.Ok) {
                        nm.notify(nid, say(context, AnswerWords.Say("Not sent", HelperWords.checkWords(check)), null))
                        return@Thread
                    }
                    post("/api/harness-sessions/$session/send", JSONObject().put("keys", JSONArray().put(key)), context, deadline)
                    keySent = true
                }
                post("/api/notifications/$id/action", JSONObject().put("action", answer), context, deadline)
                val at = LocalTime.now().format(DateTimeFormatter.ofPattern("HH:mm"))
                val helper = kind == CardKind.HelperAsks.name && session != null
                nm.notify(nid, say(context, AnswerWords.answered(helper, answer == "approve", AnswerWords.asker(title), at), null))
            } catch (e: Exception) {
                // Once the key is in, a retry must not send it again: it would land on whatever the helper asks next.
                if (keySent) nm.notify(nid, say(context, AnswerWords.sentNotRecorded, null))
                else nm.notify(nid, say(context, AnswerWords.failed, intent))
            } finally {
                pending.finish()
            }
        }.start()
    }

    private fun get(path: String, c: Context, deadline: Long): String {
        val h = URL("http://127.0.0.1:${HubService.PORT}$path").openConnection() as HttpURLConnection
        try {
            h.connectTimeout = minOf(CONNECT_MS, left(deadline))
            h.readTimeout = left(deadline)
            h.setRequestProperty("Authorization", "Bearer " + HubService.token(c))
            h.setRequestProperty("X-HQ-Client", "android")
            if (h.responseCode !in 200..299) throw java.io.IOException("status ${h.responseCode}")
            return h.inputStream.bufferedReader().use { it.readText() }
        } finally {
            h.disconnect()
        }
    }

    private fun post(path: String, body: JSONObject, c: Context, deadline: Long) {
        val h = URL("http://127.0.0.1:${HubService.PORT}$path").openConnection() as HttpURLConnection
        try {
            h.requestMethod = "POST"
            h.connectTimeout = minOf(CONNECT_MS, left(deadline))
            h.readTimeout = left(deadline)
            h.doOutput = true
            h.setRequestProperty("Authorization", "Bearer " + HubService.token(c))
            h.setRequestProperty("X-HQ-Client", "android")
            h.setRequestProperty("Content-Type", "application/json")
            h.outputStream.use { it.write(body.toString().toByteArray()) }
            if (h.responseCode !in 200..299) throw java.io.IOException("status ${h.responseCode}")
        } finally {
            h.disconnect()
        }
    }

    /** Milliseconds left of the one deadline an answer has, from the tap; none left means the answer failed. */
    private fun left(deadline: Long): Int = (deadline - SystemClock.elapsedRealtime()).toInt().also { if (it <= 0) throw java.io.IOException("no time left") }

    /** The notification that replaces the asking card: the words, and for a failure Try again (the same tap again) and Open. */
    private fun say(c: Context, words: AnswerWords.Say, retry: Intent?): Notification {
        val icon = android.graphics.drawable.Icon.createWithResource(c, HubService.statIcon(c))
        val b = Notification.Builder(c, AlertPoster.NEEDS).setSmallIcon(HubService.statIcon(c)).setContentTitle(words.title).setAutoCancel(true).setOnlyAlertOnce(true)
        words.body?.let { b.setContentText(it) }
        if (words === AnswerWords.sending) b.setProgress(0, 0, true)
        else if (retry == null) b.setTimeoutAfter(CLEAR_AFTER_MS)
        else {
            b.addAction(Notification.Action.Builder(icon, "Try again", PendingIntent.getBroadcast(c, retry.getStringExtra(ID).hashCode(), Intent(retry), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)).build())
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
        private const val DEADLINE_MS = 10_000L
        private const val CONNECT_MS = 2_000
        private const val CLEAR_AFTER_MS = 6_000L

        fun intent(c: Context, card: InboxCard, answer: String): PendingIntent {
            val i = Intent(c, AnswerReceiver::class.java).putExtra(ID, card.id).putExtra(ANSWER, answer).putExtra(SESSION, card.sessionId)
                .putExtra(KIND, card.kind.name).putExtra(TITLE, card.title).putExtra(EXCERPT, card.excerpt.joinToString("\n"))
            return PendingIntent.getBroadcast(c, (card.id + answer).hashCode(), i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
    }
}
