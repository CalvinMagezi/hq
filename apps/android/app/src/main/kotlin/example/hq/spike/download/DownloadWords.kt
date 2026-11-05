package example.hq.spike.download

import example.hq.spike.R
import example.hq.spike.ui.shell.Words

/** What a button on a download row does. */
enum class Op { Download, DownloadLeft, Pause, Resume, Cancel, TryAgain }

data class Btn(val op: Op, val label: String)

/** What a row shows for one download state: its lines, how far along it is, and its buttons. */
data class RowModel(
    val status: String? = null,
    val warn: Boolean = false,
    val progress: Float? = null,
    val detail: String? = null,
    val spinner: Boolean = false,
    val buttons: List<Btn> = emptyList(),
)

/** The words of the downloads, from the one catalogue, so a row, a notice and a test read the same sentences. */
object DownloadWords {
    fun size(w: Words, bytes: Long) = w.s(R.string.dl_mb, DownloadRules.sizeMb(bytes))

    private fun done(w: Words, bytes: Long) = w.s(R.string.dl_mb, DownloadRules.doneText(bytes))

    /** "{pct}% · {done} of {size}". */
    fun progress(w: Words, item: Item, doneBytes: Long) = w.s(R.string.dl_progress, "${DownloadRules.percent(doneBytes, item.size)}%", done(w, doneBytes), size(w, item.size))

    fun row(w: Words, item: Item, state: DlState): RowModel {
        val small = !DownloadRules.isBig(item.size)
        fun at(bytes: Long) = w.s(R.string.dl_paused_owner, done(w, bytes), size(w, item.size))
        return when (state) {
            DlState.Ready, is DlState.Asking, is DlState.NoSpace, is DlState.Cancelled ->
                RowModel(w.s(R.string.dl_size_download, size(w, item.size)), buttons = listOf(Btn(Op.Download, w.s(R.string.dl_btn_download_install))))
            DlState.WaitingWifi -> RowModel(w.s(R.string.dl_waiting), warn = true, buttons = listOf(Btn(Op.Download, w.s(R.string.dl_btn_download_size, size(w, item.size))), Btn(Op.Cancel, w.s(R.string.dl_btn_cancel))))
            is DlState.Downloading ->
                if (small) RowModel(w.s(R.string.dl_small), spinner = true)
                else RowModel(
                    w.s(if (state.onPaid) R.string.dl_over_mobile else R.string.dl_over_wifi),
                    progress = state.done.toFloat() / item.size.coerceAtLeast(1), detail = progress(w, item, state.done),
                    buttons = listOf(Btn(Op.Pause, w.s(R.string.dl_btn_pause)), Btn(Op.Cancel, w.s(R.string.dl_btn_cancel))),
                )
            is DlState.PausedWifi -> RowModel(
                w.s(R.string.dl_paused_wifi, done(w, state.done), size(w, item.size)), warn = true,
                buttons = listOf(Btn(Op.DownloadLeft, w.s(R.string.dl_btn_download_left, w.s(R.string.dl_mb, DownloadRules.leftMb(item.size, state.done))))),
            )
            is DlState.PausedOwner -> RowModel(at(state.done), buttons = listOf(Btn(Op.Resume, w.s(R.string.dl_btn_resume)), Btn(Op.Cancel, w.s(R.string.dl_btn_cancel))))
            is DlState.CutOff -> RowModel(w.s(R.string.dl_cut_off, done(w, state.done), size(w, item.size)), warn = true)
            DlState.Checking -> RowModel(w.s(R.string.dl_checking), spinner = true)
            is DlState.Done -> RowModel(if (state.paidBytes > 0) w.s(R.string.dl_data_used, DownloadRules.usedMb(state.paidBytes)) else null)
            DlState.Mismatch -> RowModel(w.s(R.string.dl_no_match), warn = true, buttons = listOf(Btn(Op.TryAgain, w.s(R.string.dl_btn_try_again))))
            is DlState.ServerFailing -> RowModel(
                w.s(R.string.dl_server_failing), warn = true,
                detail = state.done.takeIf { it > 0 }?.let { w.s(R.string.dl_kept, done(w, it), size(w, item.size)) },
                buttons = listOf(Btn(Op.TryAgain, w.s(R.string.dl_btn_try_again)), Btn(Op.Cancel, w.s(R.string.dl_btn_cancel))),
            )
        }
    }

    /** The ask's title and body for [kind]. */
    fun ask(w: Words, item: Item, kind: AskKind, doneBytes: Long): Pair<String, String> {
        val name = item.name
        val size = size(w, item.size)
        return when (kind) {
            AskKind.Mobile -> w.s(R.string.dl_ask_mobile_title) to w.s(R.string.dl_ask_mobile_body, name, size)
            AskKind.Roaming -> w.s(R.string.dl_ask_roaming_title) to w.s(R.string.dl_ask_roaming_body, name, size)
            AskKind.PaidWifi -> w.s(R.string.dl_ask_paid_wifi_title) to w.s(R.string.dl_ask_paid_wifi_body, name, size)
            AskKind.AnyNetwork -> w.s(R.string.dl_ask_any_title) to w.s(R.string.dl_ask_any_body, name, size)
            AskKind.StartOver -> w.s(R.string.dl_ask_start_over_title) to w.s(R.string.dl_ask_start_over_body, done(w, doneBytes), size)
            AskKind.WifiQuestion -> w.s(R.string.dl_wifi_q_title) to w.s(R.string.dl_wifi_q_body)
        }
    }
}
