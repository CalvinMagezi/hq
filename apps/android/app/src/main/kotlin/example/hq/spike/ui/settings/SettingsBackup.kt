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

// The backup card. Split from SettingsScreens.kt; the code is as it was.
/** The Home card that stands in for a warning on the backup row: tapping it opens Backup and about. */
@Composable
internal fun BackupSlot(slot: BackupWords.Slot, onOpen: () -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Column(
        Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Amber, shape)
            .clickable(role = Role.Button, onClick = onOpen).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(HqIcons.Warning, null, Modifier.size(22.dp), tint = Mts.Amber)
            Text(slot.title, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.semantics { heading() })
        }
        Text(slot.body, style = HqType.BodySm, color = Mts.Ink)
    }
}

/** What backups on this phone are doing, and Save a copy now: idle, running, done or failed. */
@Composable
internal fun BackupCard(ui: SettingsUi, actions: SettingsActions) {
    val w = example.hq.spike.ui.shell.rememberWords()
    val context = androidx.compose.ui.platform.LocalContext.current
    val picker = androidx.activity.compose.rememberLauncherForActivityResult(androidx.activity.result.contract.ActivityResultContracts.CreateDocument("application/zip")) { uri -> if (uri != null) actions.onSaveCopy(uri) }
    val save = { picker.launch(example.hq.spike.safe.NotesCopy.fileName(java.time.LocalDateTime.now())) }
    var details by remember { mutableStateOf<String?>(null) }
    val shape = RoundedCornerShape(12.dp)
    fun time(ms: Long) = android.text.format.DateFormat.getTimeFormat(context).format(java.util.Date(ms))
    when (val c = ui.copy) {
        is BackupCopy.State.Busy -> CardFrame(shape, false) {
            CardTitle(w.s(R.string.bk_busy_title), false)
            Text(w.s(R.string.bk_busy_body, "${c.percent}%", c.doneMb, c.totalMb), style = HqType.BodySm, color = Mts.Ink)
            val label = w.s(R.string.bk_busy_percent, c.percent)
            Box(
                Modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)).background(Mts.Surface200)
                    .semantics { contentDescription = label; progressBarRangeInfo = androidx.compose.ui.semantics.ProgressBarRangeInfo(c.percent / 100f, 0f..1f) },
            ) { Box(Modifier.fillMaxWidth(c.percent / 100f).height(4.dp).background(Mts.BrandBlue)) }
            CardButton(w.s(R.string.bk_cancel), false, actions.onSaveCopyCancel)
        }
        is BackupCopy.State.Done -> CardFrame(shape, c.onPhone) {
            CardTitle(w.s(if (c.onPhone) R.string.bk_here_too_title else R.string.bk_done_title), c.onPhone)
            Text(if (c.onPhone) w.s(R.string.bk_here_too_body) else w.s(R.string.bk_done_body, time(c.atMs)), style = HqType.BodySm, color = Mts.Ink)
            CardButton(w.s(R.string.bk_got_it), false, actions.onSaveCopyClear)
            if (c.onPhone) CardButton(w.s(R.string.bk_save_copy), true, save)
        }
        is BackupCopy.State.Failed -> CardFrame(shape, true) {
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.Top) {
                Icon(HqIcons.Warning, null, Modifier.size(22.dp), tint = Mts.Amber)
                Text(c.message, style = HqType.BodySm, color = Mts.Ink)
            }
            CardButton(w.s(R.string.bk_save_copy), true, save)
            CardButton(w.s(R.string.bk_got_it), false, actions.onSaveCopyClear)
        }
        BackupCopy.State.Idle -> {
            val card = BackupWords.card(w, ui.backup, Instant.now())
            val warn = card.tone == BackupWords.Tone.Warn
            CardFrame(shape, warn) {
                CardTitle(card.title, warn)
                Text(card.body, style = HqType.BodySm, color = Mts.Ink)
                card.said?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted) }
                card.small?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted) }
                card.lock?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted) }
                card.actions.forEach { a ->
                    when (a) {
                        BackupWords.Action.SaveCopy -> CardButton(w.s(R.string.bk_save_copy), warn, save)
                        BackupWords.Action.OpenBackground -> CardButton(w.s(R.string.bk_open_background), false) { actions.onPage(SettingsPage.Background) }
                        BackupWords.Action.GotIt -> CardButton(w.s(R.string.bk_got_it), false, actions.onGotIt)
                        BackupWords.Action.CopyDetails -> CardButton(w.s(R.string.bk_copy_details), false) { details = BackupWords.details(ui.backup, Instant.now()) }
                    }
                }
            }
        }
    }
    details?.let { CopyDetailsSheet(it) { details = null } }
}
