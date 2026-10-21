package example.hq.spike

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import example.hq.spike.ui.inbox.CardKind
import example.hq.spike.ui.helpers.AnswerCheck
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
        Thread {
            val nm = context.getSystemService(NotificationManager::class.java)
            val nid = AlertPoster.notificationId(id)
            var keySent = false
            try {
                if (kind == CardKind.HelperAsks.name && session != null) {
                    val key = if (answer == "approve") "enter" else "esc"
                    // The key goes in only if the helper's live screen still shows the dialog this notification quoted.
                    val screen = JSONObject(get("/api/harness-sessions/$session/screen?lines=$CHECK_LINES", context))
                    val lines = screen.optJSONArray("lines")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
                    val check = HelperWords.checkAnswer(HelperWords.blocked(excerpt), lines, key, screen.optString("source") == "live")
                    if (check != AnswerCheck.Ok) {
                        nm.notify(nid, result(context, "Not sent", HelperWords.checkWords(check), null))
                        return@Thread
                    }
                    post("/api/harness-sessions/$session/send", JSONObject().put("keys", JSONArray().put(key)), context)
                    keySent = true
                }
                post("/api/notifications/$id/action", JSONObject().put("action", answer), context)
                val at = LocalTime.now().format(DateTimeFormatter.ofPattern("HH:mm"))
                nm.notify(nid, result(context, if (answer == "approve") "Allowed." else "Not now. Nothing was done.", "Sent at $at.", null))
            } catch (e: Exception) {
                // Once the key is in, a retry must not send it again: it would land on whatever the helper asks next.
                if (keySent) nm.notify(nid, result(context, "Answer sent", "HQ did not record it. Open the app to check.", null))
                else nm.notify(nid, result(context, "Couldn't send your answer", "HQ didn't reply. It may be restarting.", intent))
            } finally {
                pending.finish()
            }
        }.start()
    }

    private fun get(path: String, c: Context): String {
        val h = URL("http://127.0.0.1:${HubService.PORT}$path").openConnection() as HttpURLConnection
        try {
            h.connectTimeout = TIMEOUT_MS
            h.readTimeout = TIMEOUT_MS
            h.setRequestProperty("Authorization", "Bearer " + HubService.token(c))
            h.setRequestProperty("X-HQ-Client", "android")
            if (h.responseCode !in 200..299) throw java.io.IOException("status ${h.responseCode}")
            return h.inputStream.bufferedReader().use { it.readText() }
        } finally {
            h.disconnect()
        }
    }

    private fun post(path: String, body: JSONObject, c: Context) {
        val h = URL("http://127.0.0.1:${HubService.PORT}$path").openConnection() as HttpURLConnection
        try {
            h.requestMethod = "POST"
            h.connectTimeout = TIMEOUT_MS
            h.readTimeout = TIMEOUT_MS
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

    private fun result(c: Context, title: String, body: String, retry: Intent?): Notification {
        val b = Notification.Builder(c, AlertPoster.NEEDS).setSmallIcon(HubService.statIcon(c)).setContentTitle(title).setContentText(body)
            .setAutoCancel(true).setOnlyAlertOnce(true)
        if (retry == null) b.setTimeoutAfter(CLEAR_AFTER_MS)
        else b.addAction(Notification.Action.Builder(android.graphics.drawable.Icon.createWithResource(c, HubService.statIcon(c)), "Try again",
            PendingIntent.getBroadcast(c, retry.getStringExtra(ID).hashCode(), Intent(retry), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)).build())
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
        private const val TIMEOUT_MS = 8_000
        private const val CLEAR_AFTER_MS = 6_000L

        fun intent(c: Context, card: InboxCard, answer: String): PendingIntent {
            val i = Intent(c, AnswerReceiver::class.java).putExtra(ID, card.id).putExtra(ANSWER, answer).putExtra(SESSION, card.sessionId)
                .putExtra(KIND, card.kind.name).putExtra(TITLE, card.title).putExtra(EXCERPT, card.excerpt.joinToString("\n"))
            return PendingIntent.getBroadcast(c, (card.id + answer).hashCode(), i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
    }
}
