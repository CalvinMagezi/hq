package example.hq.spike.ui.firstrun

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.R
import example.hq.spike.ui.shell.ConfirmDialog
import example.hq.spike.ui.shell.ConfirmKind
import example.hq.spike.ui.shell.ConfirmWords
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val Title = HqType.H2.copy(fontSize = 28.sp, lineHeight = 34.sp)

/** The first-run flow. It covers the shell until the last step, so nothing behind it can be reached half set up. */
@Composable
fun FirstRunHost(ui: FirstRunUi, actions: FirstRunActions) {
    val step = ui.step ?: return
    val locked = ui.busy || ui.restoring
    BackHandler(enabled = (step != Step.Welcome && step != Step.Moved) || locked) { if (!locked) actions.onBack() }
    Column(
        Modifier.fillMaxSize().background(Mts.Surface000).statusBarsPadding().navigationBarsPadding().imePadding().verticalScroll(rememberScrollState()).padding(horizontal = 24.dp, vertical = 24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        when (step) {
            Step.Welcome -> WelcomeStep(actions)
            Step.Moved -> MovedStep(ui, actions)
            Step.Model -> ModelStep(ui, actions)
            Step.Chat -> ChatStep(ui, actions)
            Step.Notify -> NotifyStep(ui, actions)
            Step.Start -> StartStep(ui, actions)
            Step.Restore -> RestoreStep(ui, actions)
        }
    }
}

@Composable
private fun WelcomeStep(actions: FirstRunActions) {
    Column(Modifier.fillMaxWidth().padding(top = 48.dp), verticalArrangement = Arrangement.spacedBy(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Image(painterResource(R.drawable.hq_mark), null, Modifier.size(88.dp))
        Text(FirstRunWords.WELCOME_TITLE, style = Title, color = Mts.Ink, textAlign = TextAlign.Center)
        Text(FirstRunWords.WELCOME_BODY, style = HqType.Body, color = Mts.InkMuted, textAlign = TextAlign.Center)
    }
    Column(Modifier.padding(top = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        GreenPill(FirstRunWords.SET_UP_NEW, onClick = actions.onNewHq)
        OutlinePill(stringResource(R.string.welcome_restore), onClick = actions.onRestore)
        Text(stringResource(R.string.welcome_moving_hint), style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(top = 8.dp))
    }
}

@Composable
fun Heading(title: String, body: String) {
    Text(title, style = Title, color = Mts.Ink)
    Text(body, style = HqType.Body, color = Mts.InkMuted)
}

@Composable
private fun ModelStep(ui: FirstRunUi, actions: FirstRunActions) {
    val context = LocalContext.current
    Heading(FirstRunWords.MODEL_TITLE, FirstRunWords.MODEL_BODY)
    val service = if (ui.moved) ui.movedFacts?.service else null
    if (service != null) Text(stringResource(R.string.moved_ai_note, service), style = HqType.BodySm, color = Mts.InkMuted)
    Text(FirstRunWords.PROVIDER_LABEL, style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Provider.entries.forEach { p ->
            val picked = p == ui.provider
            val shape = RoundedCornerShape(12.dp)
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(shape).border(if (picked) 2.dp else 1.dp, if (picked) Mts.BrandGreen else Mts.Line, shape)
                    .clickable(enabled = !ui.busy, role = Role.RadioButton) { actions.onProvider(p) }.padding(horizontal = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(p.label, style = if (picked) HqType.BodyBold else HqType.Body, color = Mts.Ink, modifier = Modifier.weight(1f))
                if (picked) Icon(HqIcons.Check, null, Modifier.size(20.dp), tint = Mts.BrandGreen)
            }
        }
    }
    if (ui.provider.needsBaseUrl) {
        PlainField(FirstRunWords.BASE_URL_LABEL, ui.baseUrl, FirstRunWords.BASE_URL_HINT, !ui.busy, KeyboardType.Uri, actions.onBaseUrl)
        PlainField(FirstRunWords.MODEL_NAME_LABEL, ui.modelName, "", !ui.busy, onChange = actions.onModelName)
    }
    SecretField(FirstRunWords.KEY_LABEL, ui.key, FirstRunWords.KEY_HINT, ui.showKey, !ui.busy, bad = ui.error != null, onShow = actions.onShowKey, onChange = actions.onKey)
    val line = ui.provider.keyLine
    val url = ui.provider.keyUrl
    if (line != null && url != null) {
        Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button) { openLink(context, url) }.padding(horizontal = 4.dp), contentAlignment = Alignment.CenterStart) {
            Text(line, style = HqType.BodySmBold, color = Mts.BrandBlue)
        }
    }
    ui.error?.let { Message(it) }
    ui.checkResult?.let { Message(it, good = it == FirstRunWords.KEY_OK) }
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Row(
            Modifier.heightIn(min = 48.dp).clip(androidx.compose.foundation.shape.CircleShape).background(Mts.Surface200).clickable(enabled = ui.key.isNotEmpty() && !ui.checking && !ui.busy, role = Role.Button, onClick = actions.onCheck).padding(horizontal = 24.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            if (ui.checking) Spinner(18)
            Text(if (ui.checking) FirstRunWords.CHECKING else FirstRunWords.CHECK_KEY, style = HqType.BodySmBold, color = Mts.Ink)
        }
        Text(FirstRunWords.CHECK_SKIP, style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.weight(1f))
    }
    GreenPill(FirstRunWords.NEXT, enabled = !ui.checking, busy = ui.busy, onClick = actions.onModelNext)
    TextAction(FirstRunWords.BACK, enabled = !ui.busy, onClick = actions.onBack)
}

@Composable
private fun ChatStep(ui: FirstRunUi, actions: FirstRunActions) {
    Heading(FirstRunWords.CHAT_TITLE, FirstRunWords.CHAT_BODY)
    SecretField(FirstRunWords.TELEGRAM_LABEL, ui.telegram, FirstRunWords.CHAT_HINT, ui.showKey, !ui.busy, onShow = actions.onShowKey, onChange = actions.onTelegram)
    SecretField(FirstRunWords.DISCORD_LABEL, ui.discord, FirstRunWords.CHAT_HINT, ui.showKey, !ui.busy, onShow = actions.onShowKey, onChange = actions.onDiscord)
    ui.error?.let { Message(it) }
    GreenPill(FirstRunWords.NEXT, busy = ui.busy, onClick = actions.onChatNext)
    Row(horizontalArrangement = Arrangement.SpaceBetween, modifier = Modifier.fillMaxWidth()) {
        TextAction(FirstRunWords.BACK, enabled = !ui.busy, onClick = actions.onBack)
        TextAction(FirstRunWords.SKIP, enabled = !ui.busy, onClick = actions.onChatSkip)
    }
}

@Composable
private fun StartStep(ui: FirstRunUi, actions: FirstRunActions) {
    if (ui.moved) {
        Text(FirstRunWords.START_TITLE, style = Title, color = Mts.Ink)
        OldPhoneWarning()
    } else {
        Heading(FirstRunWords.START_TITLE, FirstRunWords.START_BODY)
    }
    ui.error?.let { Message(it) }
    StartNotifCard()
    GreenPill(if (ui.busy) FirstRunWords.STARTING else FirstRunWords.START, busy = ui.busy, onClick = actions.onStart)
    TextAction(FirstRunWords.BACK, enabled = !ui.busy, onClick = actions.onBack)
}

@Composable
private fun RestoreStep(ui: FirstRunUi, actions: FirstRunActions) {
    val context = LocalContext.current
    val pickBackup = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> uri?.let { actions.onBackup(it, displayName(context, it)) } }
    val pickKey = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> uri?.let { actions.onKeyFile(it, displayName(context, it)) } }
    Heading(stringResource(R.string.restore_title), stringResource(R.string.restore_body))
    OutlinePill(FirstRunWords.PICK_BACKUP, enabled = !ui.restoring) { pickBackup.launch(arrayOf("*/*")) }
    val name = ui.backupName
    val size = remember(ui.backup) { ui.backup?.let { fileSize(context, it) } }
    val row = if (name != null && size != null && name.endsWith(".zip", ignoreCase = true)) stringResource(R.string.restore_file_row, name, size) else name
    Text(row ?: FirstRunWords.NO_FILE_CHOSEN, style = HqType.BodySm, color = Mts.InkMuted)
    Text(stringResource(R.string.restore_keys_again), style = HqType.BodySm, color = Mts.InkMuted)
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Choice(FirstRunWords.USE_PASSPHRASE, !ui.useKeyFile, !ui.restoring, Modifier.weight(1f)) { actions.onUseKeyFile(false) }
        Choice(FirstRunWords.USE_KEY_FILE, ui.useKeyFile, !ui.restoring, Modifier.weight(1f)) { actions.onUseKeyFile(true) }
    }
    if (ui.useKeyFile) {
        OutlinePill(FirstRunWords.PICK_KEY_FILE, enabled = !ui.restoring) { pickKey.launch(arrayOf("*/*")) }
        Text(ui.keyFileName ?: FirstRunWords.NO_FILE_CHOSEN, style = HqType.BodySm, color = Mts.InkMuted)
    } else {
        SecretField(FirstRunWords.PASSPHRASE_LABEL, ui.passphrase, FirstRunWords.PASSPHRASE_HINT, ui.showKey, !ui.restoring, onShow = actions.onShowKey, onChange = actions.onPassphrase)
    }
    ui.error?.let { Message(it) }
    if (ui.restoring) Text(FirstRunWords.RESTORING, style = HqType.BodySm, color = Mts.InkMuted)
    if (ui.restored) {
        Message(FirstRunWords.RESTORE_DONE, good = true)
        GreenPill(FirstRunWords.NEXT, onClick = actions.onContinue)
    } else {
        GreenPill(FirstRunWords.RESTORE_GO, busy = ui.restoring, onClick = actions.onRunRestore)
        TextAction(FirstRunWords.BACK, enabled = !ui.restoring, onClick = actions.onBack)
    }
}

/** Frame 2, and frame 7 when the copied database is not clean. Only the vault moved; the owner is told what did not. */
@Composable
private fun MovedStep(ui: FirstRunUi, actions: FirstRunActions) {
    val facts = ui.movedFacts
    Text(stringResource(R.string.moved_title), style = Title, color = Mts.Ink)
    when {
        ui.movedChecking || facts == null -> Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(24) }
        !facts.databaseOk -> NotClean(ui, actions)
        else -> {
            val notes = pluralStringResource(R.plurals.moved_notes, facts.notes, facts.notes)
            val tasks = pluralStringResource(R.plurals.moved_tasks, facts.tasks, facts.tasks)
            Text(stringResource(R.string.moved_count, notes, tasks), style = HqType.Body, color = Mts.InkMuted)
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Mts.Surface100).padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(stringResource(R.string.moved_keys_title), style = HqType.BodyBold, color = Mts.Ink)
                Text(stringResource(R.string.moved_keys_body), style = HqType.BodySm, color = Mts.InkMuted)
            }
            Text(stringResource(R.string.moved_fresh_title), style = HqType.BodyBold, color = Mts.Ink)
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                listOf(R.string.moved_fresh_signins, R.string.moved_fresh_outbox, R.string.moved_fresh_settings).forEach {
                    Text(stringResource(it), style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
            ui.error?.let { Message(it) }
            GreenPill(stringResource(R.string.moved_add_keys), enabled = ui.movedForgot, onClick = actions.onAddKeys)
            OutlinePill(stringResource(R.string.moved_start_fresh), enabled = !ui.busy, onClick = actions.onStartFresh)
        }
    }
    if (ui.confirmFresh) FreshDialog(actions)
}

@Composable
private fun NotClean(ui: FirstRunUi, actions: FirstRunActions) {
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).border(1.dp, Mts.Amber, RoundedCornerShape(12.dp)).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(stringResource(R.string.moved_db_title), style = HqType.BodyBold, color = Mts.Amber)
        Text(stringResource(R.string.moved_db_why), style = HqType.BodySm, color = Mts.Ink)
        Text(stringResource(R.string.moved_db_fix), style = HqType.BodySm, color = Mts.Ink)
    }
    GreenPill(stringResource(R.string.moved_db_restore), onClick = actions.onMovedRestore)
    OutlinePill(stringResource(R.string.moved_db_notes_only), enabled = !ui.movedChecking, onClick = actions.onNotesOnly)
}

@Composable
private fun FreshDialog(actions: FirstRunActions) {
    ConfirmDialog(
        ConfirmWords(
            stringResource(R.string.moved_fresh_dialog_title), stringResource(R.string.moved_fresh_dialog_body),
            stringResource(R.string.moved_keep_notes), stringResource(R.string.moved_delete_notes), stringResource(R.string.confirm_busy_deleting), ConfirmKind.Red,
        ),
        onKeep = actions.onKeepNotes, onAct = actions.onDeleteMoved,
    )
}

/** Frame 5's warning: the old phone may still be answering the same chats. */
@Composable
private fun OldPhoneWarning() {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).border(1.dp, Mts.Amber, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(stringResource(R.string.moved_old_phone_title), style = HqType.BodyBold, color = Mts.Amber)
        Text(stringResource(R.string.moved_old_phone_body), style = HqType.BodySm, color = Mts.Ink)
    }
}

@Composable
private fun Choice(text: String, picked: Boolean, enabled: Boolean, modifier: Modifier, onClick: () -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        modifier.heightIn(min = 48.dp).clip(shape).border(if (picked) 2.dp else 1.dp, if (picked) Mts.BrandGreen else Mts.Line, shape).clickable(enabled = enabled, role = Role.RadioButton, onClick = onClick).padding(horizontal = 12.dp),
        horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically,
    ) { Text(text, style = if (picked) HqType.BodySmBold else HqType.BodySm, color = Mts.Ink, textAlign = TextAlign.Center) }
}

private fun fileSize(context: Context, uri: Uri): String? = runCatching {
    context.contentResolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use { c -> if (c.moveToFirst() && !c.isNull(0)) android.text.format.Formatter.formatShortFileSize(context, c.getLong(0)) else null }
}.getOrNull()

private fun openLink(context: Context, url: String) {
    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
}

/** The file name the picker showed, so the person can see which file they chose. Never the path. */
private fun displayName(context: Context, uri: Uri): String? = runCatching {
    context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c -> if (c.moveToFirst()) c.getString(0) else null }
}.getOrNull()
