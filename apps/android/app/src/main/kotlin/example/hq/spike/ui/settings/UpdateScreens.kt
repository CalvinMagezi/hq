package example.hq.spike.ui.settings

import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.download.DlState
import example.hq.spike.download.DownloadRules
import example.hq.spike.download.DownloadWords
import example.hq.spike.download.Downloads
import example.hq.spike.download.NetKind
import example.hq.spike.download.Op
import example.hq.spike.measure.Counter
import example.hq.spike.measure.Counters
import example.hq.spike.ui.download.DownloadDialogs
import example.hq.spike.ui.download.DownloadRow
import example.hq.spike.ui.shell.rememberWords
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import example.hq.spike.update.ChannelManifest
import example.hq.spike.update.ProblemKind
import example.hq.spike.update.UpdateLogic
import example.hq.spike.update.UpdateState
import example.hq.spike.update.Updater


/** The manifest the page is about, when there is a version to talk about. */
private fun waitingManifest(s: UpdateState): ChannelManifest? = when (s) {
    is UpdateState.Installing -> null
    else -> Updater.manifestOf(s)
}

/** The version the card and the line offer: one that is ready, not put off with "Not now", and not stuck on a problem other than install permission. */
internal fun readyManifest(ui: SettingsUi): ChannelManifest? =
    waitingManifest(ui.update)?.takeIf { !ui.updateHidden && ui.update !is UpdateState.Problem || (ui.update as? UpdateState.Problem)?.kind == ProblemKind.NotAllowed }

/** The Home card for a version that is ready. It is out of the way while "Not now" holds, and gone once the version is installed. */
@Composable
internal fun UpdateCard(ui: SettingsUi, actions: SettingsActions) {
    val m = readyManifest(ui) ?: return
    val w = rememberWords()
    val shape = RoundedCornerShape(12.dp)
    Column(
        Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(stringResource(R.string.upd_card_title), style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.semantics { heading() })
        Text(if (m.size > 0) w.s(R.string.upd_card_body, m.version, DownloadWords.size(w, m.size)) else w.s(R.string.upd_card_body_no_size, m.version), style = HqType.BodySm, color = Mts.Ink)
        CardButton(stringResource(R.string.upd_card_see), true) { actions.onPage(SettingsPage.Update) }
        if (ui.backup.saveCopyOn) CardButton(stringResource(R.string.upd_card_save_copy), false) { actions.onPage(SettingsPage.About) }
    }
}

/**
 * The line under the top bar on every place while a version is ready, so nobody has to open Settings to find it. Update
 * opens the Update page; Not now puts it (and the Settings card) off for seven days, or until a newer version.
 */
@Composable
fun UpdateLine(ui: SettingsUi, onUpdate: () -> Unit, onNotNow: () -> Unit) {
    val m = readyManifest(ui) ?: return
    Row(
        Modifier.fillMaxWidth().background(Mts.Surface100).padding(start = 16.dp, end = 4.dp).heightIn(min = 48.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(8.dp).background(Mts.BrandBlue, CircleShape))
        Text(
            stringResource(R.string.upd_line_ready, m.version), style = HqType.BodySm, color = Mts.Ink,
            maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f).padding(start = 8.dp),
        )
        LineButton(stringResource(R.string.confirm_not_now), Mts.InkMuted, onNotNow)
        LineButton(stringResource(R.string.upd_line_update), Mts.BrandGreen, onUpdate)
    }
}

@Composable
private fun LineButton(label: String, ink: androidx.compose.ui.graphics.Color, onClick: () -> Unit) {
    Box(
        Modifier.heightIn(min = 48.dp).clip(RoundedCornerShape(24.dp)).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 12.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = ink) }
}

/** The card for a build that was pulled after it was installed: it says so and opens the page that has the fix. */
@Composable
internal fun WithdrawnCard(ui: SettingsUi, actions: SettingsActions) {
    val s = ui.update as? UpdateState.InstalledWithdrawn ?: return
    val context = LocalContext.current
    val shape = RoundedCornerShape(12.dp)
    Column(
        Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Amber, shape)
            .clickable(role = Role.Button) { actions.onPage(SettingsPage.Update) }.padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(UpdateLogic.wordsFor(context, s), style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.semantics { heading() })
    }
}

/** The Update page: what is on the phone, what is new, the download with its own controls, and what happens next. */
@Composable
internal fun UpdatePage(ui: SettingsUi, actions: SettingsActions) {
    val context = LocalContext.current
    val w = rememberWords()
    LaunchedEffect(Unit) { actions.onUpdatePageOpened() }
    val state = ui.update
    val facts = ui.updateFacts
    val m = waitingManifest(state)
    var sheet by remember { mutableStateOf(false) }
    Page {
        if (state is UpdateState.NeedsNewerAndroid) { TooOld(state, facts); return@Page }
        Facts(
            listOfNotNull(
                FactRow(stringResource(R.string.upd_fact_phone), w.s(R.string.upd_fact_version, facts.installedVersion, facts.installedBuild.toString())),
                m?.let { FactRow(stringResource(R.string.upd_fact_new), w.s(R.string.upd_fact_version, it.version, it.versionCode.toString())) },
                FactRow(stringResource(R.string.upd_fact_android), facts.release),
            ),
        )
        if (m == null) {
            Text(UpdateLogic.wordsFor(context, state), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
            return@Page
        }
        val notes = m.notes[java.util.Locale.getDefault().language] ?: m.notes["en"]
        if (!notes.isNullOrEmpty()) {
            Label(stringResource(R.string.upd_notes_heading))
            Column(Modifier.padding(horizontal = 20.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                notes.forEach { Text(it, style = HqType.BodySm, color = Mts.Ink) }
            }
        }
        val net = remember(ui.downloads) { example.hq.spike.download.AndroidDownloadEnv(context).net() }
        Text(
            stringResource(if (net == NetKind.Mobile || net == NetKind.Roaming) R.string.upd_net_mobile else R.string.upd_net_wifi),
            style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp),
        )
        val item = Updater.itemOf(m)
        val dl = ui.downloads[item.id] ?: DlState.Ready
        val engine = { Downloads.get(item.id) }
        val busy = state is UpdateState.Downloading || dl !is DlState.Ready && dl !is DlState.Done && dl !is DlState.Cancelled && dl !is DlState.Mismatch
        if (busy || dl is DlState.Mismatch || dl is DlState.ServerFailing) {
            DownloadRow(item, dl, { op ->
                val e = engine()
                when (op) {
                    Op.Pause -> e?.pause()
                    Op.Resume -> e?.resume()
                    Op.Cancel -> e?.cancel()
                    Op.TryAgain -> e?.tryAgain()
                    Op.Download, Op.DownloadLeft -> e?.request()
                }
            })
        } else {
            val label = if (state is UpdateState.Installing) null else stringResource(R.string.dl_btn_download_install)
            Text(UpdateLogic.wordsFor(context, state), style = HqType.BodySm, color = if (state is UpdateState.Problem) Mts.Amber else Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
            if (label != null) Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                CardButton(label, true) {
                    if (!facts.canInstall) sheet = true
                    actions.onInstallUpdate(m)
                }
                CardButton(stringResource(R.string.upd_not_now), false) { actions.onUpdateNotNow(); actions.onBack() }
            }
        }
        DownloadDialogs(item, dl, engine)
        if (state is UpdateState.Busy) {
            example.hq.spike.update.InstallBusyDialog(state.reason, onWait = { actions.onWaitThenInstall(state.manifest) }, onStop = { actions.onStopAndInstall(state.manifest) }, onDismiss = { actions.onBusyDismiss(state.manifest) })
        }
        if (state is UpdateState.Problem && state.kind == ProblemKind.NotAllowed) {
            CardButton(stringResource(R.string.upd_sheet_open), false) { sheet = true }
        }
        Text(stringResource(R.string.upd_leave_page), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
        Text(stringResource(R.string.upd_footnote), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
    }
    if (sheet) InstallPermissionSheet { sheet = false }
}

@Composable
private fun TooOld(state: UpdateState.NeedsNewerAndroid, facts: Updater.Facts) {
    val context = LocalContext.current
    val number = UpdateLogic.androidNumber(state.minSdk)
    val body = if (number != null) stringResource(R.string.upd_old_body, state.version, number, facts.release, facts.installedVersion)
    else stringResource(R.string.upd_old_body_unnamed, state.version, facts.release, facts.installedVersion)
    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text(stringResource(R.string.upd_old_title), style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.semantics { heading() })
        Text(body, style = HqType.BodySm, color = Mts.Ink)
        Text(stringResource(R.string.upd_old_note), style = HqType.BodySm, color = Mts.InkMuted)
    }
}

/** What to do once: allow AgentHQ to install apps in Android's own settings, then come back. */
@Composable
private fun InstallPermissionSheet(onClose: () -> Unit) {
    val context = LocalContext.current
    androidx.compose.material3.AlertDialog(
        onDismissRequest = onClose,
        title = { Text(stringResource(R.string.upd_sheet_title), style = HqType.BodyBold, color = Mts.Ink) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(stringResource(R.string.upd_sheet_body), style = HqType.BodySm, color = Mts.Ink)
                Text("1. " + stringResource(R.string.upd_sheet_step_1), style = HqType.BodySm, color = Mts.Ink)
                Text("2. " + stringResource(R.string.upd_sheet_step_2), style = HqType.BodySm, color = Mts.Ink)
                Text("3. " + stringResource(R.string.upd_sheet_step_3), style = HqType.BodySm, color = Mts.Ink)
            }
        },
        confirmButton = {
            androidx.compose.material3.TextButton({
                onClose()
                context.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + context.packageName)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.upd_sheet_open), style = HqType.BodyBold, color = Mts.BrandBlue) }
        },
        dismissButton = { androidx.compose.material3.TextButton(onClose, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.upd_not_now), style = HqType.BodyBold, color = Mts.InkMuted) } },
        containerColor = Mts.Surface100,
    )
}

/** Spending: how large downloads treat mobile data, what HQ knows about this phone's Wi-Fi, and what HQ has used. */
@Composable
internal fun MobileData(ui: SettingsUi, actions: SettingsActions) {
    val context = LocalContext.current
    val w = rememberWords()
    val used = Counters.get(context).snapshot().current
    fun mb(c: Counter) = DownloadRules.doneText(used.slot(c, 0))
    Label(stringResource(R.string.dl_set_group))
    Group {
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Switch) { actions.onAskAnyNetwork(!ui.askAnyNetwork) }.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                Text(stringResource(R.string.dl_set_ask_any), style = HqType.BodyBold, color = Mts.Ink)
                Text(stringResource(R.string.dl_set_ask_any_sub), style = HqType.BodySm, color = Mts.InkMuted)
            }
            Switch(ui.askAnyNetwork, null)
        }
        Divider()
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                Text(stringResource(R.string.dl_set_known_title), style = HqType.BodyBold, color = Mts.Ink)
                Text(w.s(R.string.dl_set_known_line, ui.wifiFree.toString(), ui.wifiPaid.toString()), style = HqType.BodySm, color = Mts.InkMuted)
            }
            if (ui.wifiFree + ui.wifiPaid > 0) androidx.compose.material3.TextButton(actions.onForgetWifi, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.dl_set_forget), style = HqType.BodyBold, color = Mts.BrandBlue) }
        }
    }
    Label(stringResource(R.string.dl_set_data_title))
    Facts(
        listOf(
            FactRow(stringResource(R.string.dl_set_data_updates), w.s(R.string.dl_mb, mb(Counter.DataAppUpdates))),
            FactRow(stringResource(R.string.dl_set_data_speech), w.s(R.string.dl_mb, mb(Counter.DataSpeechModel))),
            FactRow(stringResource(R.string.dl_set_data_server), w.s(R.string.dl_mb, mb(Counter.DataServerChecks))),
        ),
    )
}

/** Backup and about: the unfinished downloads and a Remove that Undo can take back. */
@Composable
internal fun StorageGroup(ui: SettingsUi, actions: SettingsActions) {
    if (ui.unfinishedBytes <= 0) return
    val w = rememberWords()
    Label(stringResource(R.string.dl_set_storage_title))
    Group {
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(w.s(R.string.dl_set_storage_line, w.s(R.string.dl_mb, DownloadRules.sizeMb(ui.unfinishedBytes))), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.weight(1f).padding(vertical = 8.dp))
            androidx.compose.material3.TextButton(actions.onRemoveUnfinished, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.dl_set_remove), style = HqType.BodyBold, color = Mts.DangerText) }
        }
    }
}
