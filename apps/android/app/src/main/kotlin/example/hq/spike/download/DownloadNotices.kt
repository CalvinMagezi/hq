package example.hq.spike.download

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import example.hq.spike.AlertPoster
import example.hq.spike.HubService
import example.hq.spike.R
import example.hq.spike.ui.shell.Words

/** The Downloads notices: silent, ongoing while a download runs, one per download, with the buttons its state has. */
object DownloadNotices {
    const val ACTION = "example.hq.spike.DOWNLOAD_ACTION"
    const val EXTRA_ID = "download_id"
    const val EXTRA_OP = "download_op"
    const val OP_PAUSE = "pause"
    const val OP_RESUME = "resume"
    const val OP_CANCEL = "cancel"
    const val OP_UNDO = "undo"
    private const val ID_BASE = 7_000
    private const val ID_SPAN = 900
    private const val MIN_GAP_MS = 1_000L

    private val last = java.util.concurrent.ConcurrentHashMap<String, Pair<Long, Int>>()

    /** What a notice says for [state]: title, body, ongoing and its buttons. Null when the state has no notice. */
    data class Spec(val title: String, val body: String?, val ongoing: Boolean, val progress: Int?, val ops: List<Pair<String, String>>)

    fun spec(w: Words, item: Item, state: DlState): Spec? {
        val name = if (item.speech) w.s(R.string.dl_n_running_speech) else w.s(R.string.dl_n_running, item.name)
        val pause = OP_PAUSE to w.s(R.string.dl_btn_pause)
        val cancel = OP_CANCEL to w.s(R.string.dl_btn_cancel)
        return when (state) {
            is DlState.Downloading -> Spec(name, DownloadWords.progress(w, item, state.done), true, DownloadRules.percent(state.done, item.size), listOf(pause, cancel))
            is DlState.Cancelled -> Spec(w.s(R.string.dl_cancelled), item.name, false, null, listOf(OP_UNDO to w.s(R.string.undo)))
            DlState.WaitingWifi -> Spec(w.s(R.string.dl_waiting), w.s(R.string.dl_n_waiting_body, item.name, DownloadWords.size(w, item.size)), false, null, listOf(cancel))
            is DlState.PausedWifi -> Spec(w.s(R.string.dl_n_paused_title), w.s(R.string.dl_n_paused_body, item.name, doneText(w, state.done), DownloadWords.size(w, item.size)), false, null, listOf(cancel))
            is DlState.CutOff -> Spec(w.s(R.string.dl_n_cut_title), w.s(R.string.dl_n_cut_body, item.name, doneText(w, state.done), DownloadWords.size(w, item.size)), false, null, emptyList())
            else -> null
        }
    }

    private fun doneText(w: Words, bytes: Long) = w.s(R.string.dl_mb, DownloadRules.doneText(bytes))

    /** Posts or clears the notice for a change of state; a running one is updated at most about once a second and only when the percent moves. */
    fun onChange(c: Context, item: Item, state: DlState) {
        val nm = c.getSystemService(NotificationManager::class.java) ?: return
        val id = ID_BASE + Math.floorMod(item.id.hashCode(), ID_SPAN)
        val w = Words.of(c)
        val spec = spec(w, item, state)
        if (spec == null) { nm.cancel(id); last.remove(item.id); return }
        if (state is DlState.Downloading) {
            val now = System.currentTimeMillis()
            val before = last[item.id]
            val pct = spec.progress ?: 0
            if (before != null && (now - before.first < MIN_GAP_MS || before.second == pct)) return
            last[item.id] = now to pct
        } else {
            last.remove(item.id)
        }
        val b = Notification.Builder(c, AlertPoster.DOWNLOADS).setSmallIcon(HubService.statIcon(c)).setContentTitle(spec.title)
            .setOngoing(spec.ongoing).setOnlyAlertOnce(true).setAutoCancel(!spec.ongoing)
        spec.body?.let { b.setContentText(it) }
        spec.progress?.let { b.setProgress(100, it, false) }
        for ((op, label) in spec.ops) {
            val intent = Intent(c, HubService::class.java).setAction(ACTION).putExtra(EXTRA_ID, item.id).putExtra(EXTRA_OP, op)
            val pending = PendingIntent.getService(c, (item.id + op).hashCode(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            b.addAction(Notification.Action.Builder(null, label, pending).build())
        }
        nm.notify(id, b.build())
    }
}
