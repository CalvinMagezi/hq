package example.hq.spike.update

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import example.hq.spike.AlertPoster
import example.hq.spike.HubService
import example.hq.spike.R
import example.hq.spike.download.DownloadWords
import example.hq.spike.ui.shell.Words

/** The update notices on the App updates channel: silent, one found notice per version, then a reminder, and each tap opens the update page. */
object UpdateNotices {
    enum class Kind { Found, Reminder }

    const val LINK = "hq://settings/update"
    private const val DAY_MS = 24L * 60 * 60 * 1000
    private const val REMIND_MS = 7 * DAY_MS
    private const val ID = 7_950

    /** Whether to post now: the first time a version is seen, then once a week, and an urgent one once a day. [firstAt] and [lastAt] are 0 until posted. */
    fun due(firstAt: Long, lastAt: Long, now: Long, urgent: Boolean): Kind? = when {
        firstAt <= 0 -> Kind.Found
        now < lastAt -> null
        now - lastAt >= (if (urgent) DAY_MS else REMIND_MS) -> Kind.Reminder
        else -> null
    }

    fun found(c: Context, m: ChannelManifest, kind: Kind) {
        val w = Words.of(c)
        val body = if (kind == Kind.Found) {
            w.s(R.string.upd_n_found_body, m.version, DownloadWords.size(w, m.size))
        } else {
            w.s(R.string.upd_n_remind_body, m.version)
        }
        post(c, w.s(R.string.upd_n_found_title), body)
    }

    fun downloaded(c: Context, m: ChannelManifest) {
        val w = Words.of(c)
        post(c, w.s(R.string.upd_n_downloaded_title, m.version), w.s(R.string.upd_n_downloaded_body), w.s(R.string.upd_n_install))
    }

    fun installed(c: Context, version: String) {
        val w = Words.of(c)
        post(c, w.s(R.string.upd_n_installed_title, version), w.s(R.string.upd_n_installed_body))
    }

    fun withdrawn(c: Context, version: String) {
        val w = Words.of(c)
        post(c, w.s(R.string.update_withdrawn_title, version), w.s(R.string.upd_n_withdrawn_body))
    }

    fun clear(c: Context) = c.getSystemService(NotificationManager::class.java)?.cancel(ID)

    private fun post(c: Context, title: String, body: String, action: String? = null) {
        val tap = AlertPoster.open(c, LINK)
        val b = Notification.Builder(c, AlertPoster.APP_UPDATES).setSmallIcon(HubService.statIcon(c)).setContentTitle(title).setContentText(body)
            .setContentIntent(tap).setAutoCancel(true).setOnlyAlertOnce(true)
        if (action != null) b.addAction(Notification.Action.Builder(null, action, tap).build())
        c.getSystemService(NotificationManager::class.java)?.notify(ID, b.build())
    }
}
