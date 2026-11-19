package example.hq.spike.ui.settings

import example.hq.spike.ui.shell.AttentionKind
import example.hq.spike.ui.shell.AttentionOrInline
import example.hq.spike.ui.shell.SnackbarRow
import example.hq.spike.ui.shell.rememberAttentionKey
import example.hq.spike.R
import example.hq.spike.ui.firstrun.FirstRunWords
import example.hq.spike.ui.firstrun.StoredKeysPage
import androidx.activity.compose.BackHandler
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import java.time.Instant
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.Lifecycle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.runtime.produceState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.measure.Counters
import example.hq.spike.measure.CellWords
import example.hq.spike.measure.Noticed
import example.hq.spike.measure.NoticeLine
import example.hq.spike.update.UpdateLogic
import example.hq.spike.update.Updater
import example.hq.spike.update.UpdateState
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.shell.ConfirmCopy
import example.hq.spike.ui.shell.ConfirmDialog
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.UndoSnackbar
import example.hq.spike.ui.shell.consumeTouches
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.webhost.LocalOpenWeb
import example.hq.spike.AlertPoster
import example.hq.spike.LocalNotif
import example.hq.spike.NotifOrigin
import example.hq.spike.ui.webhost.WebPage
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.delay

// What HQ noticed, and the update group. Split from SettingsScreens.kt; the code is as it was.
/** The counters, this version and the last, read only from the counter file: it works with HQ stopped. */
@Composable
internal fun NoticedPage(counters: Counters) {
    val context = androidx.compose.ui.platform.LocalContext.current
    var snap by remember { mutableStateOf(counters.snapshot()) }
    var line by remember { mutableStateOf<NoticeLine?>(null) }
    var askClear by remember { mutableStateOf(false) }
    val config = LocalConfiguration.current
    val narrow = config.screenWidthDp <= NARROW_DP && config.fontScale >= 2f
    val words = CellWords({ a, b -> context.getString(R.string.noticed_of, a.toString(), b.toString()) }, stringResource(R.string.noticed_yes), stringResource(R.string.noticed_no))
    val asksPermission = android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU
    val notAsked = stringResource(R.string.noticed_not_asked)
    Page {
        Text(stringResource(R.string.noticed_intro), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp))
        if (!snap.on) Box(Modifier.padding(top = 12.dp)) { Notice(stringResource(R.string.noticed_off_note, Noticed.day(snap.stoppedOn)), HqIcons.Info, warn = true) }
        Row(Modifier.fillMaxWidth().padding(start = 32.dp, top = 16.dp, end = 32.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.noticed_since, Noticed.day(snap.current.since)), style = HqType.BodySmBold, color = Mts.InkMuted, modifier = Modifier.weight(1f))
            Text(snap.current.version, style = HqType.BodySmBold, color = Mts.Ink, textAlign = TextAlign.End, modifier = Modifier.width(CELL_WIDTH))
            if (!narrow) snap.previous?.let { Text(it.version, style = HqType.BodySmBold, color = Mts.InkMuted, textAlign = TextAlign.End, modifier = Modifier.width(CELL_WIDTH)) }
        }
        if (narrow) snap.previous?.let { Text(stringResource(R.string.noticed_narrow_caption, it.version), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 32.dp, end = 32.dp)) }
        Noticed.lines.groupBy { it.group }.forEach { (group, rows) ->
            Label(stringResource(group))
            Group {
                rows.forEachIndexed { i, l ->
                    if (i > 0) Divider()
                    val skipped = l.asksPermission && !asksPermission
                    val now = if (skipped) notAsked else Noticed.cell(snap.current, l, words)
                    val before = if (skipped) null else snap.previous?.let { Noticed.cell(it, l, words) }
                    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button) { line = l }.padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(stringResource(l.label), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.weight(1f))
                        if (narrow) {
                            Column(horizontalAlignment = Alignment.End) {
                                Text(now, style = HqType.BodySmBold, color = Mts.Ink, textAlign = TextAlign.End)
                                before?.let { Text(it, style = SmallText, color = Mts.InkMuted, textAlign = TextAlign.End) }
                            }
                        } else {
                            Text(now, style = HqType.BodySmBold, color = Mts.Ink, textAlign = TextAlign.End, modifier = if (skipped) Modifier.widthIn(max = VALUE_MAX) else Modifier.width(CELL_WIDTH))
                            if (!skipped) before?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.End, modifier = Modifier.width(CELL_WIDTH)) }
                        }
                    }
                }
            }
        }
        Label(stringResource(R.string.noticed_switch))
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Switch) { counters.setOn(!snap.on); snap = counters.snapshot() }.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.noticed_switch), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(if (snap.on) R.string.noticed_switch_on else R.string.noticed_switch_off), style = HqType.BodySm, color = Mts.InkMuted)
                }
                Switch(
                    checked = snap.on, onCheckedChange = null,
                    colors = SwitchDefaults.colors(checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen, checkedBorderColor = Mts.BrandGreen, uncheckedTrackColor = Mts.Surface200, uncheckedThumbColor = Mts.Line, uncheckedBorderColor = Mts.Line),
                )
            }
            Divider()
            Column(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { askClear = true }.padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.Center) {
                Text(stringResource(R.string.noticed_clear), style = HqType.BodyBold, color = Mts.Ink)
                Text(stringResource(R.string.noticed_clear_sub), style = HqType.BodySm, color = Mts.InkMuted)
            }
        }
        Text(stringResource(R.string.noticed_foot), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp))
    }
    line?.let { l ->
        BackHandler { line = null }
        Sheet({ line = null }) {
            Text(stringResource(l.label), style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(bottom = 8.dp))
            Text(stringResource(l.what), style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(bottom = 8.dp))
            Text(stringResource(l.only), style = HqType.BodySm, color = Mts.InkMuted)
            Row(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalArrangement = Arrangement.End) { DialogButton(stringResource(R.string.noticed_done)) { line = null } }
        }
    }
    if (askClear) {
        BackHandler { askClear = false }
        Box(Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = SCRIM_ALPHA)).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) { askClear = false })
            Column(Modifier.align(Alignment.Center).padding(24.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(Mts.Surface200).padding(24.dp)) {
                Text(stringResource(R.string.noticed_clear_title), style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(bottom = 12.dp))
                Text(stringResource(R.string.noticed_clear_body), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(bottom = 20.dp))
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                    DialogButton(stringResource(R.string.noticed_clear_keep)) { askClear = false }
                    DialogButton(stringResource(R.string.noticed_clear_confirm)) { counters.clear(); snap = counters.snapshot(); askClear = false }
                }
            }
        }
    }
}

/** The App updates rows: one that opens the Update page when a version is waiting (or checks when none is), and the test-versions switch. */
@Composable
/** [busyDialog] is false where the page draws the busy dialog itself, so one install question never shows twice. */
internal fun UpdateGroup(ui: SettingsUi, actions: SettingsActions, checkRow: Boolean = true, busyDialog: Boolean = true) {
    val state = ui.update
    val context = androidx.compose.ui.platform.LocalContext.current
    val working = state == UpdateState.Checking
    val waiting = Updater.manifestOf(state) != null && state !is UpdateState.Installing
    Group {
        if (checkRow) Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(enabled = !working, role = Role.Button) { if (waiting) actions.onPage(SettingsPage.Update) else actions.onCheckUpdate() }.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                Text(stringResource(if (waiting) R.string.upd_row_install else R.string.upd_row_check), style = HqType.BodyBold, color = Mts.Ink)
                Text(UpdateLogic.wordsFor(context, state), style = HqType.BodySm, color = if (state is UpdateState.Problem || state is UpdateState.Failed) Mts.Amber else Mts.InkMuted)
            }
            if (working) Spinner(20) else Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
        }
        if (busyDialog && state is UpdateState.Busy) {
            example.hq.spike.update.InstallBusyDialog(
                state.reason,
                onWait = { actions.onWaitThenInstall(state.manifest) },
                onStop = { actions.onStopAndInstall(state.manifest) },
                onDismiss = { actions.onBusyDismiss(state.manifest) },
            )
        }
        if (checkRow) Divider()
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Switch) { actions.onUpdateMain(!ui.updateMain) }.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                Text(stringResource(R.string.settings_follow_every_build_of), style = HqType.BodyBold, color = Mts.Ink)
                Text(stringResource(if (ui.updateMain) R.string.settings_update_on_line else R.string.settings_update_off_line), style = HqType.BodySm, color = Mts.InkMuted)
            }
            Text(stringResource(if (ui.updateMain) R.string.settings_update_toggle_on else R.string.settings_update_toggle_off), style = HqType.BodySmBold, color = Mts.Ink)
        }
    }
}
