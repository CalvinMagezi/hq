package example.hq.spike.ui.download

import android.content.Intent
import android.os.storage.StorageManager
import android.provider.Settings
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import example.hq.spike.R
import example.hq.spike.download.AskKind
import example.hq.spike.download.DlState
import example.hq.spike.download.DownloadRules
import example.hq.spike.download.DownloadWords
import example.hq.spike.download.Item
import example.hq.spike.download.LargeDownload
import example.hq.spike.ui.shell.rememberWords
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** Two equal outlined buttons side by side; they stack when the text is large or a label is long. Wait for Wi-Fi comes first. */
@Composable
private fun EqualChoices(first: String, second: String, onFirst: () -> Unit, onSecond: () -> Unit, stackedAlways: Boolean = false) {
    val stacked = stackedAlways || DownloadRules.stacks(first, second, LocalDensity.current.fontScale)
    val one: @Composable (String, () -> Unit, Modifier) -> Unit = { label, click, mod ->
        OutlinedButton(click, mod.heightIn(min = 48.dp)) { Text(label, style = HqType.BodyBold, color = Mts.BrandBlue) }
    }
    if (stacked) {
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            one(first, onFirst, Modifier.fillMaxWidth())
            one(second, onSecond, Modifier.fillMaxWidth())
        }
    } else {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            one(first, onFirst, Modifier.weight(1f))
            one(second, onSecond, Modifier.weight(1f))
        }
    }
}

@Composable
private fun Card(content: @Composable () -> Unit) {
    Column(Modifier.clip(RoundedCornerShape(12.dp)).background(Mts.Surface200).padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) { content() }
}

/** "Download on mobile data?" and its siblings: what it costs in the owner's words, then Wait for Wi-Fi or Download · {size}. Back is nothing fetched. */
@Composable
fun AskDataDialog(item: Item, kind: AskKind, done: Long, onWait: () -> Unit, onDownload: () -> Unit, onDismiss: () -> Unit) {
    val w = rememberWords()
    val (title, body) = DownloadWords.ask(w, item, kind, done)
    Dialog(onDismissRequest = onDismiss) {
        Card {
            Text(title, style = HqType.ScreenTitle, color = Mts.Ink)
            Text(body, style = HqType.Body, color = Mts.Ink)
            EqualChoices(stringResource(R.string.dl_ask_wait), stringResource(R.string.dl_ask_download, DownloadWords.size(w, item.size)), onWait, onDownload)
        }
    }
}

/** "Is this Wi-Fi paid by the MB?": asked once for each Wi-Fi. Back asks again next time. */
@Composable
fun WifiQuestionDialog(onPaid: () -> Unit, onFree: () -> Unit, onDismiss: () -> Unit) {
    Dialog(onDismissRequest = onDismiss) {
        Card {
            Text(stringResource(R.string.dl_wifi_q_title), style = HqType.ScreenTitle, color = Mts.Ink)
            Text(stringResource(R.string.dl_wifi_q_body), style = HqType.Body, color = Mts.Ink)
            EqualChoices(stringResource(R.string.dl_wifi_paid), stringResource(R.string.dl_wifi_free), onPaid, onFree)
        }
    }
}

/** The phone cannot hold the file: says how much to free and opens Android's own storage screen. */
@Composable
fun NoSpaceDialog(state: DlState.NoSpace, onNotNow: () -> Unit) {
    val w = rememberWords()
    val context = LocalContext.current
    Dialog(onDismissRequest = onNotNow) {
        Card {
            Text(stringResource(R.string.dl_space_title), style = HqType.ScreenTitle, color = Mts.Ink)
            Text(stringResource(R.string.dl_space_body, DownloadWords.size(w, state.short)), style = HqType.Body, color = Mts.Ink)
            Text(stringResource(R.string.dl_space_need, DownloadWords.size(w, state.need)), style = HqType.Body, color = Mts.InkMuted)
            EqualChoices(stringResource(R.string.dl_space_not_now), stringResource(R.string.dl_space_open), onNotNow, {
                try { context.startActivity(Intent(StorageManager.ACTION_MANAGE_STORAGE).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } catch (_: android.content.ActivityNotFoundException) {
                    context.startActivity(Intent(Settings.ACTION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                }
                onNotNow()
            }, stackedAlways = true)
            Text(stringResource(R.string.dl_space_hint), style = HqType.BodySm, color = Mts.InkMuted)
        }
    }
}

/** Whichever dialog [state] needs for [engine]; nothing for the other states. */
@Composable
fun DownloadDialogs(item: Item, state: DlState, engine: () -> LargeDownload?) {
    when (state) {
        is DlState.Asking ->
            if (state.kind == AskKind.WifiQuestion) WifiQuestionDialog({ engine()?.answerWifi(true) }, { engine()?.answerWifi(false) }, { engine()?.dismissAsk() })
            else AskDataDialog(item, state.kind, state.done, { engine()?.waitForWifi() }, { engine()?.accept() }, { engine()?.dismissAsk() })
        is DlState.NoSpace -> NoSpaceDialog(state) { engine()?.discard() }
        else -> {}
    }
}
