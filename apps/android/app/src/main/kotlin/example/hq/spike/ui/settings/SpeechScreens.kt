package example.hq.spike.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.download.DlState
import example.hq.spike.download.DownloadRules
import example.hq.spike.download.Downloads
import example.hq.spike.download.perform
import example.hq.spike.measure.Counter
import example.hq.spike.measure.Counters
import example.hq.spike.ui.download.DownloadDialogs
import example.hq.spike.ui.download.DownloadRow
import example.hq.spike.ui.shell.rememberWords
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import example.hq.spike.voice.VoiceModels

/** The "Speech model" section of Phone assistant: what it is, where it came from, what it cost in mobile data, and Delete. Before the download it offers it. */
@Composable
internal fun SpeechSection(ui: SettingsUi, actions: SettingsActions) {
    val context = LocalContext.current
    val w = rememberWords()
    Column {
        if (ui.speechBytes >= 0) {
            val paid = Counters.get(context).snapshot().current.slot(Counter.DataSpeechModel, 0)
            val rows = buildList {
                add(FactRow(stringResource(R.string.speech_on_phone), w.s(R.string.dl_mb, DownloadRules.sizeMb(ui.speechBytes))))
                add(FactRow(stringResource(R.string.speech_language), stringResource(R.string.speech_language_value)))
                add(FactRow(stringResource(R.string.speech_from), stringResource(R.string.speech_from_value)))
                if (paid > 0) add(FactRow(stringResource(R.string.speech_data), w.s(R.string.speech_data_value, DownloadRules.usedMb(paid))))
            }
            Facts(rows)
            Text(stringResource(R.string.speech_note), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
            Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                CardButton(stringResource(R.string.speech_delete_act), false) { actions.onAskDeleteSpeech(true) }
            }
        } else {
            val item = remember { VoiceModels(context.applicationContext).item() }
            val dl = ui.downloads[item.id] ?: DlState.Ready
            DownloadRow(item, dl, { op -> Downloads.engine(context, item).perform(op) })
            DownloadDialogs(item, dl) { Downloads.get(item.id) }
        }
    }
}
