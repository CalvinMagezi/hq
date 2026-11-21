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

internal val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private const val LINE_MS = 4_000L
private const val REFRESH_MS = 15_000L
private const val PAIR_POLL_MS = 3_000L
private val BAR_HEIGHT = 8.dp
private const val DISABLED_ALPHA = 0.5f
internal const val SCRIM_ALPHA = 0.62f
internal val VALUE_MAX = 180.dp
private val KEY_COLUMN = 120.dp

internal fun icon(key: String): ImageVector = when (key) {
    "background" -> HqIcons.Power
    "battery" -> HqIcons.Battery
    "notifications" -> HqIcons.Inbox
    "capture" -> HqIcons.Plus
    "key" -> HqIcons.Key
    "computer" -> HqIcons.Plug
    "update" -> HqIcons.Download
    "technical" -> HqIcons.Info
    "assistant" -> HqIcons.Mic
    "calendar" -> HqIcons.Clock
    "contacts" -> HqIcons.Phone
    "finance" -> HqIcons.Gauge
    "chat" -> HqIcons.Chat
    "phone" -> HqIcons.Phone
    "shield" -> HqIcons.Shield
    "gauge" -> HqIcons.Gauge
    HQ_CODER_ROW -> HqIcons.Code
    SUGGESTS_ROW -> HqIcons.Spark
    WORK_TASKS_ROW -> HqIcons.Tasks
    WORK_HELPERS_ROW -> HqIcons.Helpers
    else -> HqIcons.Archive
}

@Composable
private fun pageTitle(ui: SettingsUi): String = when (ui.page) {
    SettingsPage.SetupApp -> stringResource(R.string.settings_page_setup_app, SettingsWords.appName(ui.setup?.platform ?: "telegram"))
    SettingsPage.PairApp -> stringResource(R.string.settings_page_pair_app, SettingsWords.appName(ui.pair?.platform ?: "telegram"))
    else -> title(ui.page)
}

@Composable
internal fun title(page: SettingsPage) = when (page) {
    SettingsPage.Home -> stringResource(R.string.settings_title)
    SettingsPage.Safety -> stringResource(R.string.row_safety)
    SettingsPage.Keys -> stringResource(R.string.row_ai_keys)
    SettingsPage.Messaging -> stringResource(R.string.row_messaging)
    SettingsPage.Devices -> stringResource(R.string.devices_title)
    SettingsPage.Spending -> stringResource(R.string.row_spending)
    SettingsPage.About -> stringResource(R.string.settings_backup_title)
    SettingsPage.Background -> stringResource(R.string.about_background)
    SettingsPage.KeepRunning -> stringResource(R.string.settings_page_keep_running)
    SettingsPage.Notifications -> stringResource(R.string.row_notifications)
    SettingsPage.Origins -> stringResource(R.string.settings_page_origins)
    SettingsPage.Computers -> stringResource(R.string.settings_page_computers)
    SettingsPage.CaptureWidgets -> stringResource(R.string.settings_page_capture)
    SettingsPage.SetupApp -> stringResource(R.string.settings_page_setup)
    SettingsPage.PairApp -> stringResource(R.string.dev_pair)
    SettingsPage.Technical -> stringResource(R.string.settings_technical_title)
    SettingsPage.Noticed -> stringResource(R.string.noticed_title)
    SettingsPage.Update -> stringResource(R.string.upd_page_title)
    SettingsPage.Assistant -> stringResource(R.string.assist_row_title)
    SettingsPage.Calendar -> stringResource(R.string.cal_row_title)
    SettingsPage.Contacts -> stringResource(R.string.contacts_row_title)
    SettingsPage.Finance -> stringResource(R.string.finance_row_title)
    SettingsPage.Suggests -> stringResource(R.string.row_suggests)
}

/** Settings over the whole shell, with its own pages: Back goes up one page, then closes. */
@Composable
fun SettingsHost(ui: SettingsUi, actions: SettingsActions) {
    val owner = LocalLifecycleOwner.current
    // One loop for as long as Settings is open and the app is in front: closing Settings ends it, and opening it again starts one.
    LaunchedEffect(ui.open) {
        if (ui.open) owner.lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) { actions.onRefresh(); delay(REFRESH_MS) }
        }
    }
    if (!ui.open) return
    BackHandler(onBack = actions.onBack)
    Box(Modifier.fillMaxSize().background(Mts.Surface000).consumeTouches().statusBarsPadding().navigationBarsPadding()) {
        Column(Modifier.fillMaxSize()) {
            Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 4.dp, end = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                IconAction(HqIcons.ArrowBack, stringResource(R.string.notify_back), iconSize = 24.dp, onClick = actions.onBack)
                Text(pageTitle(ui), style = HqType.ScreenTitle, color = Mts.Ink, modifier = Modifier.padding(start = 4.dp))
            }
            when (ui.page) {
                SettingsPage.Home -> Home(ui, actions)
                SettingsPage.Safety -> Safety(ui, actions)
                SettingsPage.Keys -> Keys(ui, actions)
                SettingsPage.Messaging -> Messaging(ui, actions)
                SettingsPage.Devices -> Devices(ui, actions)
                SettingsPage.Spending -> Spending(ui, actions)
                SettingsPage.About -> About(ui, actions)
                SettingsPage.Background -> Background(ui, actions)
                SettingsPage.KeepRunning -> KeepRunning(ui, actions)
                SettingsPage.Notifications -> Notifications(ui, actions)
                SettingsPage.CaptureWidgets -> CaptureWidgets(ui, actions)
                SettingsPage.SetupApp -> SetupApp(ui, actions)
                SettingsPage.PairApp -> PairApp(ui, actions)
                SettingsPage.Technical -> Technical(ui, actions)
                SettingsPage.Update -> UpdatePage(ui, actions)
                SettingsPage.Assistant -> AssistantPage(ui, actions)
                SettingsPage.Calendar -> CalendarPage()
                SettingsPage.Contacts -> ContactsPage()
                SettingsPage.Finance -> FinancePage()
                SettingsPage.Suggests -> SuggestsPage()
                SettingsPage.Noticed -> NoticedPage(Counters.get(androidx.compose.ui.platform.LocalContext.current))
                SettingsPage.Origins -> ReadOnlyList(ui, stringResource(R.string.settings_none_origins)) { SettingsWords.originRows(ui.snap.settings).forEach { MonoRow(it) } }
                SettingsPage.Computers -> ComputersAndTools(ui, actions.onSectionConsumed)
            }
        }
        val context = androidx.compose.ui.platform.LocalContext.current
        ui.askMode?.let { toActive ->
            val w = SettingsWords.modeDialog(example.hq.spike.ui.shell.rememberWords(), toActive)
            ConfirmDialog(if (toActive) ConfirmCopy.start(context, w.title, w.body) else ConfirmCopy.pause(context, w.title, w.body), actions.onModeCancel, actions.onModeConfirm)
        }
        ui.keySheet?.let { KeySheetView(it, actions) }
        ui.askDisconnect?.let { p -> ConfirmDialog(ConfirmCopy.disconnectApp(context, SettingsWords.appName(p)), { actions.onAskDisconnect(null) }, actions.onDisconnect) }
        ui.askReplace?.let { p -> ConfirmDialog(ConfirmCopy.unlinkAccount(context, SettingsWords.appName(p)), { actions.onAskReplace(null) }, { actions.onPair(p, true) }) }
        ui.keyRemove?.let { q ->
            ConfirmDialog(ConfirmCopy.removeKey(context, ConfirmCopy.keyLabel(q.name), q.only), { actions.onAskRemoveKey(null) }, actions.onRemoveKey, busy = q.busy, onExtra = actions.onReplaceKey)
        }
        if (ui.askDeleteSpeech) ConfirmDialog(ConfirmCopy.deleteSpeech(context), { actions.onAskDeleteSpeech(false) }, actions.onDeleteSpeech)
        if (ui.askSignOutAll) ConfirmDialog(ConfirmCopy.signOutAll(context), { actions.onSignOutAll(false) }, actions.onSignOutAllConfirm)
        ui.askSignOut?.let { d -> ConfirmDialog(ConfirmCopy.signOutDevice(context, d.label), { actions.onAskSignOut(null) }, actions.onSignOutConfirm, busy = ui.busy == "out:${d.id}", error = ui.signOutError) }
        if (ui.askServerDisconnect) ConfirmDialog(ConfirmCopy.disconnectServer(context, ConfirmCopy.hostOf(ui.notif.server.address)), { actions.onAskServerDisconnect(false) }, actions.onServerDisconnect)
        if (ui.askRestart) {
            val w = SettingsWords.restartWords(example.hq.spike.ui.shell.rememberWords())
            ConfirmDialog(ConfirmCopy.restart(context, w.title, w.body), { actions.onRestart(false) }, actions.onRestartConfirm)
        }
        ui.undoLine?.let { UndoSnackbar(it, actions.onUndoDiscard, Modifier.align(Alignment.BottomCenter), actions.onUndoExpire) }
        ui.line?.let { Snack(it, actions.onDismissLine, Modifier.align(Alignment.BottomCenter)) }
        ui.dlUndo?.let { UndoSnackbar(stringResource(if (it == DlUndo.Forget) R.string.dl_set_forgot else R.string.dl_set_removed), actions.onDlUndo, Modifier.align(Alignment.BottomCenter), actions.onDlUndoExpire) }
    }
}

@Composable
internal fun Snack(text: String, onDismiss: () -> Unit, modifier: Modifier) {
    AttentionOrInline(AttentionKind.Snackbar, rememberAttentionKey(), modifier, onDismiss, restartOn = text) { m -> SnackbarRow(text, null, {}, m) }
}

/** A row that opens one of the app's own full screens (pairing). */
@Composable
private fun OpenScreen(text: String, screen: Class<*>) {
    val context = androidx.compose.ui.platform.LocalContext.current
    Text(text, style = HqType.BodyBold, color = Mts.BrandBlue, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable { context.startActivity(android.content.Intent(context, screen)) }.padding(horizontal = 20.dp, vertical = 16.dp))
}

private const val HIGHLIGHT_ANIM_MS = 220

/** Every (page, target) pair whose UI registers a [sectionTarget]; shared with placement/consistency tests. */
internal val WIRED_SECTION_TARGETS: Set<Pair<SettingsPage, ScrollTarget>> = setOf(
    SettingsPage.Keys to ScrollTarget.KeysModel,
    SettingsPage.Keys to ScrollTarget.KeysWhichAnswers,
    SettingsPage.Keys to ScrollTarget.KeysYourKeys,
    SettingsPage.Keys to ScrollTarget.KeysStored,
    SettingsPage.Assistant to ScrollTarget.AssistantRightNow,
    SettingsPage.Assistant to ScrollTarget.AssistantSpeech,
    SettingsPage.Background to ScrollTarget.BackgroundRightNow,
    SettingsPage.Background to ScrollTarget.BackgroundKeepRunning,
    SettingsPage.Safety to ScrollTarget.SafetyAllowedAddresses,
    SettingsPage.Safety to ScrollTarget.SafetyTrustedSites,
    SettingsPage.Computers to ScrollTarget.ComputersList,
    SettingsPage.Computers to ScrollTarget.ComputersTools,
)

internal class SectionScroller(
    val scrollState: ScrollState,
    val machine: SectionScrollMachine,
)

@Composable
internal fun rememberSectionScroller(ui: SettingsUi, onSectionConsumed: () -> Unit = {}): SectionScroller {
    val target = SettingsScroll.target(ui.page, ui.section)
    val scrollState = remember(ui.page) { ScrollState(0) }
    val context = androidx.compose.ui.platform.LocalContext.current
    val reducedMotion = remember(context) {
        android.provider.Settings.Global.getFloat(context.contentResolver, android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f
    }
    val machine = remember(ui.page, target, reducedMotion) {
        SectionScrollMachine(target, reducedMotion)
    }
    val scroller = remember(scrollState, machine) { SectionScroller(scrollState, machine) }
    LaunchedEffect(machine, machine.layoutTick) {
        machine.runIfReady(
            scrollTo = { y -> if (reducedMotion) scrollState.scrollTo(y) else scrollState.animateScrollTo(y) },
            onConsumed = onSectionConsumed,
        )
    }
    return scroller
}

@Composable
internal fun Modifier.sectionTarget(scroller: SectionScroller?, section: ScrollTarget?): Modifier {
    if (scroller == null || section == null) return this
    val active = scroller.machine.highlighted == section
    val bg = if (scroller.machine.reducedMotion) {
        if (active) Mts.Surface200 else Color.Transparent
    } else {
        animateColorAsState(
            if (active) Mts.Surface200 else Color.Transparent,
            animationSpec = tween(HIGHLIGHT_ANIM_MS),
            label = "sectionHighlight",
        ).value
    }
    return this
        .onGloballyPositioned { scroller.machine.register(section, it.positionInRoot().y + scroller.scrollState.value) }
        .clip(RoundedCornerShape(8.dp))
        .background(bg)
}

@Composable
internal fun Page(scroller: SectionScroller? = null, content: @Composable () -> Unit) {
    val scroll = scroller?.scrollState ?: rememberScrollState()
    Column(
        Modifier
            .fillMaxSize()
            .onGloballyPositioned { if (scroller != null) scroller.machine.setRootTop(it.positionInRoot().y.toInt()) }
            .verticalScroll(scroll)
            .padding(bottom = 24.dp),
    ) { content() }
}

@Composable
internal fun Label(text: String, modifier: Modifier = Modifier) = Text(
    text.uppercase(),
    style = HqType.Group.copy(letterSpacing = 0.06.em),
    color = Mts.InkMuted,
    modifier = modifier.padding(start = 16.dp, top = 20.dp, end = 16.dp, bottom = 4.dp),
)

@Composable
internal fun Group(content: @Composable () -> Unit) {
    Column(Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Mts.Surface100)) { content() }
}

@Composable
internal fun Divider() = Box(Modifier.fillMaxWidth().size(0.dp, 1.dp).background(Mts.Hairline))

@Composable
internal fun Notice(text: String, icon: ImageVector = HqIcons.Lock, warn: Boolean = false) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, if (warn) Mts.Amber else Mts.Hairline, shape).padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top,
    ) {
        Icon(icon, null, Modifier.size(22.dp), tint = if (warn) Mts.Amber else Mts.InkMuted)
        Text(text, style = HqType.BodySm, color = Mts.Ink)
    }
}

@Composable
private fun NavRow(row: HomeRow, first: Boolean, onClick: () -> Unit) {
    if (!first) Divider()
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon(row.key), null, Modifier.size(24.dp), tint = Mts.Ink)
        Column(Modifier.weight(1f)) {
            Text(row.title, style = HqType.BodyBold, color = Mts.Ink)
            Text(row.status, style = HqType.BodySm, color = if (row.warn) Mts.Amber else Mts.InkMuted)
        }
        if (row.warn) Icon(HqIcons.Warning, null, Modifier.size(20.dp), tint = Mts.Amber)
        Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

@Composable
private fun Fact(row: FactRow, first: Boolean, onOpen: (SettingsPage) -> Unit = {}, modifier: Modifier = Modifier) {
    if (!first) Divider()
    val tap = row.opens?.let { page -> Modifier.heightIn(min = 48.dp).clickable(role = Role.Button) { onOpen(page) } } ?: Modifier
    Column(Modifier.fillMaxWidth().then(modifier).then(tap).padding(horizontal = 16.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(row.title, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
            Text(row.value, style = HqType.BodySm, color = if (row.good) Mts.BrandGreen else Mts.InkMuted, textAlign = TextAlign.End, modifier = Modifier.widthIn(max = VALUE_MAX))
            if (row.opens != null) Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
        }
        row.text?.let { Text(it, style = SmallText, color = Mts.InkMuted) }
    }
}

@Composable
internal fun Facts(rows: List<FactRow>, onOpen: (SettingsPage) -> Unit = {}, scroller: SectionScroller? = null, targetFor: (FactRow) -> ScrollTarget? = { null }) =
    Group { rows.forEachIndexed { i, r -> Fact(r, i == 0, onOpen, Modifier.sectionTarget(scroller, targetFor(r))) } }

@Composable
private fun Waiting(ui: SettingsUi) {
    if (!ui.loaded) Text(stringResource(R.string.settings_asking_hq), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
}

/** True when the page has nothing to show yet: it says it is asking, or why there is nothing (no answer, or an error answer). */
@Composable
private fun Unanswered(ui: SettingsUi, route: String, have: Boolean): Boolean {
    if (have) return false
    when {
        !ui.loaded -> Waiting(ui)
        route in ui.snap.unavailable -> Notice(stringResource(R.string.settings_page_unavailable), HqIcons.Info)
        else -> NoAnswer()
    }
    return true
}

@Composable
private fun NoAnswer(text: String = stringResource(R.string.settings_no_answer_stopped)) =
    Notice(text, HqIcons.WifiOff, warn = true)

// ---------------------------------------------------------------- home

@Composable
internal fun SettingsSearchField(query: String, onQuery: (String) -> Unit, modifier: Modifier = Modifier) {
    val label = stringResource(R.string.settings_search_hint)
    val clearLabel = stringResource(R.string.search_clear)
    val keyboard = LocalSoftwareKeyboardController.current
    val focusManager = LocalFocusManager.current
    val shape = RoundedCornerShape(24.dp)
    Row(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .heightIn(min = 48.dp)
            .clip(shape)
            .background(Mts.Surface100)
            .border(1.dp, Mts.Line, shape)
            .padding(start = 16.dp, end = if (query.isNotEmpty()) 4.dp else 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(HqIcons.Search, label, Modifier.size(20.dp), tint = Mts.InkMuted)
        BasicTextField(
            value = query,
            onValueChange = onQuery,
            singleLine = true,
            textStyle = HqType.Body.copy(color = Mts.Ink),
            cursorBrush = SolidColor(Mts.Ink),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = {
                keyboard?.hide()
                focusManager.clearFocus()
            }),
            modifier = Modifier.weight(1f).padding(vertical = 10.dp),
            decorationBox = { inner ->
                Box {
                    if (query.isEmpty()) Text(label, style = HqType.Body, color = Mts.InkMuted)
                    inner()
                }
            },
        )
        if (query.isNotEmpty()) {
            Box(
                Modifier
                    .size(48.dp)
                    .clickable(role = Role.Button, onClickLabel = clearLabel) { onQuery("") }
                    .semantics { contentDescription = clearLabel },
                contentAlignment = Alignment.Center,
            ) {
                Icon(HqIcons.Close, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
        }
    }
}

@Composable
internal fun SettingsSearchResultRow(label: String, first: Boolean, onClick: () -> Unit) {
    if (!first) Divider()
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label }
            .padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
        Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

@Composable
internal fun Home(ui: SettingsUi, actions: SettingsActions) {
    val w = example.hq.spike.ui.shell.rememberWords()
    val context = androidx.compose.ui.platform.LocalContext.current
    val strip = ui.savedAt?.takeIf { ui.stale }?.let { w.s(R.string.bk_strip_saved, android.text.format.DateFormat.getTimeFormat(context).format(java.util.Date(it.toEpochMilli()))) }
    val keepToFix = ui.phone.firstOrNull { it.page == SettingsPage.Background }?.warn == true
    val ready = readyManifest(ui)
    val card = SettingsWords.homeCard(ui.update is UpdateState.InstalledWithdrawn, if (keepToFix) PhoneWords.keepRunning(w, ui.keep, ui.now).checked.count { !it.ok } else 0, BackupWords.warns(ui.backup, ui.now), ready != null)
    val updates = SettingsWords.updatesHome(w, ui.update, ui.updateFacts.installedVersion, ready?.version)
    val index = remember(w) { SettingsIndex.build(w) }
    val keyboard = LocalSoftwareKeyboardController.current
    val focusManager = LocalFocusManager.current
    Page {
        SettingsSearchField(ui.searchQuery, actions.onSearchQuery)
        if (ui.searchQuery.isNotBlank()) {
            val results = SettingsSearch.results(ui.searchQuery, index)
            if (results.isEmpty()) {
                Text(
                    stringResource(R.string.settings_search_empty),
                    style = HqType.BodySm,
                    color = Mts.InkMuted,
                    modifier = Modifier.padding(horizontal = 20.dp, vertical = 16.dp).semantics { liveRegion = LiveRegionMode.Polite },
                )
            } else {
                Text(
                    w.n(R.plurals.settings_search_count, results.size, results.size),
                    style = HqType.BodySm,
                    color = Mts.InkMuted,
                    modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp, bottom = 4.dp).semantics { liveRegion = LiveRegionMode.Polite },
                )
                Group {
                    results.forEachIndexed { i, entry ->
                        SettingsSearchResultRow(SettingsIndex.rowLabel(w, entry), i == 0) {
                            keyboard?.hide()
                            focusManager.clearFocus()
                            if (entry.page != null) actions.onPageSection(entry.page, entry.sectionKey)
                            else actions.onPhoneRow(entry.rowKey)
                        }
                    }
                }
            }
            return@Page
        }
        // One card at most: Home never stacks a warning on a warning, and the plain Update card gives way to any of them.
        when (card) {
            SettingsWords.HomeCard.Withdrawn -> WithdrawnCard(ui, actions)
            SettingsWords.HomeCard.KeepStops -> PhoneWords.keepRunning(w, ui.keep, ui.now).let { page -> KeepStopsCard(page.noticeTitle, page.notice) { actions.onPage(SettingsPage.Background); actions.onPage(SettingsPage.KeepRunning) } }
            SettingsWords.HomeCard.Backup -> BackupWords.slot(w, ui.backup, ui.now)?.let { slot -> BackupSlot(slot) { actions.onPage(SettingsPage.About) } }
            SettingsWords.HomeCard.Update -> UpdateCard(ui, actions)
            SettingsWords.HomeCard.None -> Unit
        }
        SettingsWords.home(w, ui.snap, ui.now, ui.phone, ui.loaded, ui.backup, strip, serverRow(w, context), updates).forEachIndexed { gi, g ->
            Text(g.title.uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = if (gi == 0) 8.dp else 20.dp, end = 16.dp, bottom = 4.dp))
            g.lead?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 4.dp)) }
            Group {
                g.rows.forEachIndexed { i, r -> NavRow(r, i == 0) { if (r.page != null) actions.onPage(r.page) else actions.onPhoneRow(r.key) } }
            }
            g.note?.let { Text(it, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp)) }
        }
    }
}

/** The warning card for "Android may stop HQ": what is wrong in one sentence, tap to open the checks that fix it. */
@Composable
private fun KeepStopsCard(title: String, body: String, onOpen: () -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Column(
        Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Amber, shape)
            .clickable(role = Role.Button, onClick = onOpen).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(HqIcons.Warning, null, Modifier.size(22.dp), tint = Mts.Amber)
            Text(title, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.semantics { heading() })
        }
        Text(body, style = HqType.BodySm, color = Mts.Ink)
    }
}

/** Computers and Connected tools, once the two tabs of Safety and limits: one page, two sections, read only. */
@Composable
private fun ComputersAndTools(ui: SettingsUi, onSectionConsumed: () -> Unit = {}) {
    val w = example.hq.spike.ui.shell.rememberWords()
    val scroller = rememberSectionScroller(ui, onSectionConsumed)
    Page(scroller) {
        Notice(stringResource(R.string.settings_read_only_notice))
        if (ui.snap.settings == null) { Unanswered(ui, "settings", false); return@Page }
        Label(stringResource(R.string.settings_section_computers), Modifier.sectionTarget(scroller, ScrollTarget.ComputersList))
        val computers = SettingsWords.computerRows(w, ui.snap.settings)
        if (computers.isEmpty()) Notice(stringResource(R.string.settings_none_computers), HqIcons.Info) else Facts(computers)
        Label(stringResource(R.string.settings_tools_connected), Modifier.sectionTarget(scroller, ScrollTarget.ComputersTools))
        val tools = SettingsWords.toolRows(w, ui.snap.settings)
        if (tools.isEmpty()) Notice(stringResource(R.string.settings_none_tools), HqIcons.Info) else Facts(tools)
    }
}

// ---------------------------------------------------------------- safety and limits

@Composable
private fun Safety(ui: SettingsUi, actions: SettingsActions) {
    val w = example.hq.spike.ui.shell.rememberWords()
    val scroller = rememberSectionScroller(ui, actions.onSectionConsumed)
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SettingsWords.SafetyTab.entries.forEach { t ->
                val on = t == ui.safetyTab
                Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).selectable(selected = on, role = Role.Tab, onClick = { actions.onTab(t) }), contentAlignment = Alignment.Center) {
                    Box(
                        Modifier.heightIn(min = 36.dp).clip(CircleShape).then(if (on) Modifier.background(Mts.BrandGreen, CircleShape).border(1.dp, Mts.BrandGreen, CircleShape) else Modifier.border(1.dp, Mts.Line, CircleShape)).padding(horizontal = 14.dp),
                        contentAlignment = Alignment.Center,
                    ) { Text(stringResource(t.label), style = HqType.BodySmBold, color = if (on) Mts.OnGreen else Mts.Ink, maxLines = 1) }
                }
            }
        }
        Page(scroller) {
            Notice(stringResource(R.string.settings_read_only_notice))
            val rows = SettingsWords.safetyRows(w, ui.snap.settings, ui.safetyTab)
            Box(Modifier.padding(top = 12.dp)) {
                if (rows.isEmpty()) Unanswered(ui, "settings", false)
                else Facts(rows, actions.onPage, scroller) { if (it.opens == SettingsPage.Origins) ScrollTarget.SafetyAllowedAddresses else null }
            }
            if (ui.safetyTab == SettingsWords.SafetyTab.Safety && ui.trustedSites.isNotEmpty()) TrustedSites(ui.trustedSites, actions.onForgetSite, scroller)
            if (ui.safetyTab == SettingsWords.SafetyTab.Helpers && rows.isNotEmpty()) HelperPolicy(SettingsWords.helperPolicy(ui.snap.settings))
        }
    }
}

/** Which helper questions reach the owner: three choices, the current one marked. Read only until the app can write HQ's settings. */
@Composable
private fun HelperPolicy(current: SettingsWords.HelperPolicy?) {
    Label(stringResource(R.string.settings_helper_policy_title))
    Text(stringResource(R.string.settings_helper_policy_intro), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 4.dp, end = 20.dp, bottom = 8.dp))
    Group {
        SettingsWords.HelperPolicy.entries.forEachIndexed { i, choice ->
            if (i > 0) Divider()
            Row(
                Modifier.fillMaxWidth().heightIn(min = 64.dp).selectable(selected = choice == current, enabled = false, role = Role.RadioButton, onClick = {}).padding(horizontal = 16.dp, vertical = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                androidx.compose.material3.RadioButton(
                    selected = choice == current, onClick = null,
                    colors = androidx.compose.material3.RadioButtonDefaults.colors(selectedColor = Mts.BrandGreen, unselectedColor = Mts.Line, disabledSelectedColor = Mts.BrandGreen, disabledUnselectedColor = Mts.Line),
                )
                Column(Modifier.weight(1f)) {
                    Text(stringResource(choice.title), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(choice.about), style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
        }
    }
    if (current == null) Text(stringResource(R.string.settings_helper_policy_unknown), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
    Text(stringResource(R.string.settings_helper_policy_note), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
}

@Composable
private fun TrustedSites(sites: List<String>, onForget: (String) -> Unit, scroller: SectionScroller? = null) {
    Label(stringResource(R.string.sites_trusted_title), Modifier.sectionTarget(scroller, ScrollTarget.SafetyTrustedSites))
    Text(stringResource(R.string.sites_trusted_body), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 20.dp))
    sites.forEach { site ->
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 20.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(site, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
            example.hq.spike.ui.helpers.OutlineButton(stringResource(R.string.sites_trusted_remove)) { onForget(site) }
        }
    }
}

// ---------------------------------------------------------------- AI service and keys

@Composable
private fun Keys(ui: SettingsUi, actions: SettingsActions) {
    val w = example.hq.spike.ui.shell.rememberWords()
    val s = ui.snap.settings
    val scroller = rememberSectionScroller(ui, actions.onSectionConsumed)
    Page(scroller) {
        // HQ's part of the page needs HQ; the keys on this phone do not, so they show whatever HQ says.
        if (SettingsWords.KeysPart.Hq in SettingsWords.keysParts(s) && s != null) {
            Label(stringResource(R.string.settings_label_model), Modifier.sectionTarget(scroller, ScrollTarget.KeysModel))
            Facts(listOf(FactRow(w.s(R.string.settings_which_ai_uses), SettingsWords.activeModel(w, s)), FactRow(w.s(R.string.settings_local_only), SettingsWords.onOff(w, s.optJSONObject("model")?.optBoolean("local_only") == true))))
            Text(stringResource(R.string.settings_the_model_is_set), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
            val ai = SettingsWords.aiRows(w, ui.snap)
            if (ai.isNotEmpty()) {
                Label(stringResource(R.string.settings_label_which_answers), Modifier.sectionTarget(scroller, ScrollTarget.KeysWhichAnswers))
                Facts(ai)
                Text(stringResource(R.string.settings_hq_tries_them_in), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 8.dp, end = 20.dp))
            }
            Label(stringResource(R.string.settings_label_your_keys), Modifier.sectionTarget(scroller, ScrollTarget.KeysYourKeys))
            Group {
                SettingsWords.keyRows(s, ui.snap.copilot, ui.snap.providers).forEachIndexed { i, (name, set, source) ->
                    if (i > 0) Divider()
                    Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(name, style = HqType.BodyBold, color = Mts.Ink)
                            Text(stringResource(if (set) (if (source == "env") R.string.settings_key_set_env else R.string.settings_key_set) else if (source == "unreadable") R.string.settings_key_unreadable else R.string.settings_key_not_set), style = HqType.BodySm, color = if (set) Mts.BrandGreen else Mts.InkMuted)
                        }
                        SettingsWords.keyAction(w, s, name, set, source)?.let { a ->
                            Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).clickable(role = Role.Button) { actions.onKeyOpen(KeySheet(a.id, name, replace = set)) }.padding(horizontal = 16.dp), contentAlignment = Alignment.Center) {
                                Text(a.label, style = HqType.BodySmBold, color = Mts.Ink)
                            }
                        }
                    }
                }
            }
            if (ai.isEmpty()) {
                Label(stringResource(R.string.settings_label_first_down))
                Group {
                    SettingsWords.fallbackRows(w, s).forEachIndexed { i, (a, b) ->
                        if (i > 0) Divider()
                        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text(a, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
                            Text(b, style = HqType.BodySm, color = Mts.InkMuted)
                        }
                    }
                }
            }
        } else Unanswered(ui, "settings", false)
        Label(FirstRunWords.KEYS_TITLE, Modifier.sectionTarget(scroller, ScrollTarget.KeysStored))
        StoredKeysPage(ui, actions)
    }
}

// ---------------------------------------------------------------- messaging apps

@Composable
private fun Messaging(ui: SettingsUi, actions: SettingsActions) {
    val w = example.hq.spike.ui.shell.rememberWords()
    Page {
        if (Unanswered(ui, "chatApps", ui.snap.chatApps != null)) return@Page
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            listOf("telegram" to "Telegram", "discord" to "Discord").forEach { (key, name) -> AppCardView(SettingsWords.appCard(w, ui.snap.chatApps, key, name, ui.now), key, ui.restarting, actions) }
        }
        Text(stringResource(if (ui.restarting) R.string.settings_restarting_dots else R.string.settings_changes_restart), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
    }
}

@Composable
private fun AppCardView(c: SettingsWords.AppCard, platform: String, busy: Boolean, actions: SettingsActions) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(40.dp).background(Mts.Surface200, CircleShape), contentAlignment = Alignment.Center) { Icon(HqIcons.Chat, null, Modifier.size(22.dp), tint = Mts.Ink) }
            Column {
                Text(c.name, style = HqType.ScreenTitle, color = Mts.Ink)
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(8.dp).background(if (c.running) Mts.BrandGreen else if (c.on) Mts.Amber else Mts.Line, CircleShape))
                    Text(c.line, style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
        }
        if (c.on) {
            c.bot?.let { KeyValue(stringResource(R.string.settings_bot), it) }
            KeyValue(stringResource(R.string.settings_your_account), c.owner, if (c.linked) Mts.BrandGreen else Mts.InkMuted)
            if (!c.linked) GreenButton(stringResource(R.string.settings_link_my, c.name), enabled = !busy) { actions.onPair(platform, false) }
            Row(Modifier.offset(x = (-12).dp), horizontalArrangement = Arrangement.spacedBy(0.dp)) {
                if (c.linked) TextLink(stringResource(R.string.settings_link_different), Mts.Ink, enabled = !busy) { actions.onAskReplace(platform) }
                TextLink(stringResource(R.string.settings_disconnect), Mts.Ink, enabled = !busy) { actions.onAskDisconnect(platform) }
            }
        } else {
            Text(stringResource(R.string.settings_add_bot, c.name), style = HqType.BodySm, color = Mts.InkMuted)
            GreenButton(stringResource(R.string.assist_card_set_up), enabled = !busy) { actions.onSetup(platform) }
        }
    }
}

@Composable
internal fun GreenButton(label: String, enabled: Boolean = true, busy: Boolean = false, modifier: Modifier = Modifier.fillMaxWidth(), onClick: () -> Unit) {
    Row(
        modifier.heightIn(min = 48.dp).clip(CircleShape).background(if (enabled) Mts.BrandGreen else Mts.Surface200).clickable(enabled = enabled, role = Role.Button, onClick = onClick),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically,
    ) {
        if (busy) Spinner(18, Mts.OnGreen)
        Text(label, style = HqType.BodySmBold, color = if (enabled) Mts.OnGreen else Mts.InkMuted)
    }
}

/** The Standby warning above the steps and the code: nothing listens while background work is paused. */
@Composable
private fun StandbyNotice(platform: String, onOpen: () -> Unit) {
    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(0.dp)) {
        Notice(SettingsWords.standbyNotice(example.hq.spike.ui.shell.rememberWords(), platform), HqIcons.Warning, warn = true)
        TextLink(stringResource(R.string.bk_open_background), Mts.BrandBlue, onClick = onOpen)
    }
}

@Composable
private fun SetupApp(ui: SettingsUi, actions: SettingsActions) {
    val setup = ui.setup ?: return
    val platform = setup.platform
    val context = androidx.compose.ui.platform.LocalContext.current
    val clipboard = androidx.compose.ui.platform.LocalClipboardManager.current
    var typed by remember { androidx.compose.runtime.mutableStateOf("") }
    var okFor by remember { androidx.compose.runtime.mutableStateOf<String?>(null) }
    LaunchedEffect(setup.found) { if (setup.found != null) okFor = typed.trim() }
    val checked = setup.found != null && okFor == typed.trim() && typed.isNotBlank()
    val field = RoundedCornerShape(12.dp)
    Page {
        if (ui.mode != "active") StandbyNotice(platform) { actions.onPage(SettingsPage.Background) }
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            SettingsWords.setupSteps(example.hq.spike.ui.shell.rememberWords(), platform).forEachIndexed { i, text ->
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top) {
                    Box(Modifier.size(28.dp).background(Mts.Surface200, CircleShape), contentAlignment = Alignment.Center) { Text("${i + 1}", style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.Ink) }
                    Text(text, style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.padding(top = 4.dp))
                }
            }
            Box(
                Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button) { runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(SettingsWords.setupUrl(platform))).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)) } }.padding(horizontal = 24.dp),
                contentAlignment = Alignment.Center,
            ) { Text(SettingsWords.setupLabel(example.hq.spike.ui.shell.rememberWords(), platform), style = HqType.BodySmBold, color = Mts.Ink) }
            Row(
                Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(field).background(Mts.Surface100).border(1.dp, Mts.Line, field).padding(start = 16.dp, top = 8.dp, end = 8.dp, bottom = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_bot_token), style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
                    androidx.compose.foundation.text.BasicTextField(
                        typed, { typed = it }, Modifier.fillMaxWidth(), enabled = !setup.saving, singleLine = true,
                        textStyle = HqType.Body.copy(color = Mts.Ink, fontFamily = FontFamily.Monospace, letterSpacing = 0.1.em), cursorBrush = androidx.compose.ui.graphics.SolidColor(Mts.Ink),
                        visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation(),
                        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.Password, autoCorrectEnabled = false),
                    )
                }
                IconAction(HqIcons.Paste, stringResource(R.string.settings_paste_token), iconSize = 22.dp) { clipboard.getText()?.text?.let { typed = it.trim() } }
            }
            if (checked) Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(HqIcons.Check, null, Modifier.size(20.dp), tint = Mts.BrandGreen)
                Text(stringResource(R.string.settings_found_bot, setup.found.orEmpty()), style = HqType.BodySmBold, color = Mts.Ink)
            }
            if (checked && setup.intentOff) Text(SettingsWords.intentOff(example.hq.spike.ui.shell.rememberWords()), style = HqType.BodySm, color = Mts.Amber)
            if (!checked) setup.error?.let { Text(it, style = HqType.BodySm, color = Mts.DangerText) }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Row(
                    Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).clickable(enabled = typed.isNotBlank() && !setup.checking && !setup.saving, role = Role.Button) { actions.onCheckToken(typed) }.padding(horizontal = 24.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (setup.checking) Spinner(18)
                    Text(stringResource(if (setup.checking) R.string.settings_checking_dots else R.string.settings_check_token), style = HqType.BodySmBold, color = Mts.Ink)
                }
                GreenButton(stringResource(if (setup.saving) R.string.settings_restarting_btn else R.string.settings_save_restart), enabled = checked && !setup.saving, busy = setup.saving, modifier = Modifier.weight(1f)) { actions.onSaveToken(typed) }
            }
        }
    }
}

@Composable
private fun PairApp(ui: SettingsUi, actions: SettingsActions) {
    val pair = ui.pair ?: return
    val platform = pair.platform
    val context = androidx.compose.ui.platform.LocalContext.current
    val left by produceState(pair.expiresAt - Instant.now().epochSecond, pair.code) {
        while (true) { value = (pair.expiresAt - Instant.now().epochSecond).coerceAtLeast(0); delay(1_000) }
    }
    LaunchedEffect(pair.code) { while (true) { actions.onPairPoll(); delay(PAIR_POLL_MS) } }
    val expired = left <= 0
    val w = example.hq.spike.ui.shell.rememberWords()
    Page {
        if (ui.mode != "active") StandbyNotice(platform) { actions.onPage(SettingsPage.Background) }
        Column(Modifier.fillMaxWidth().padding(start = 16.dp, top = 24.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Text(SettingsWords.pairBlurb(w, platform), style = HqType.Body, color = Mts.InkMuted, textAlign = TextAlign.Center)
            val shape = RoundedCornerShape(12.dp)
            Row(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(start = 20.dp, top = 12.dp, end = 8.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(pair.code, style = HqType.H2.copy(fontFamily = FontFamily.Monospace, fontSize = 34.sp, lineHeight = 44.sp, letterSpacing = 0.06.em), color = if (expired) Mts.InkMuted else Mts.Ink, modifier = Modifier.weight(1f))
                IconAction(HqIcons.Copy, stringResource(R.string.settings_copy_code), iconSize = 22.dp) { actions.onCopy(pair.code) }
            }
            if (!expired) {
                if (pair.deepLink != null) GreenButton(stringResource(R.string.settings_open_in, SettingsWords.appName(platform))) { runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(pair.deepLink)).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)) } }
                Text(SettingsWords.pairHelp(w, platform, pair.command, pair.deepLink != null), style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.Center)
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                    Spinner(18)
                    Text(SettingsWords.pairWaiting(w, SettingsWords.clock(left)), style = HqType.BodySm, color = Mts.InkMuted)
                }
            } else {
                Text(w.s(R.string.settings_pair_expired), style = HqType.BodySm, color = Mts.InkMuted)
                GreenButton(stringResource(R.string.settings_get_new_code)) { actions.onPair(platform, false) }
            }
            ui.setup?.appId?.takeIf { platform == "discord" }?.let { id ->
                Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button) { runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(SettingsWords.inviteUrl(id))).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)) } }.padding(horizontal = 24.dp), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.settings_open_the_invite_link), style = HqType.BodySmBold, color = Mts.Ink)
                }
                if (ui.setup?.intentOff == true) Text(SettingsWords.intentOff(w), style = HqType.BodySm, color = Mts.Amber, textAlign = TextAlign.Center)
            }
            TextLink(stringResource(R.string.bk_cancel), Mts.BrandBlue, onClick = actions.onBack)
        }
    }
}

@Composable
private fun KeyValue(key: String, value: String, color: Color = Mts.Ink) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(key, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.width(KEY_COLUMN))
        Text(value, style = HqType.BodySm, color = color)
    }
}

/** A word that acts, in a colour, with a 48 dp target. */
@Composable
internal fun TextLink(text: String, color: Color, enabled: Boolean = true, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
        Text(text, style = HqType.BodySmBold, color = if (enabled) color else Mts.InkMuted)
    }
}

@Composable
private fun Devices(ui: SettingsUi, actions: SettingsActions) {
    val code = ui.deviceCode
    val left by produceState(0L, code) {
        while (code != null) { value = (code.expiresAt - Instant.now().epochSecond).coerceAtLeast(0); delay(1_000) }
    }
    Page {
        // Pairing does not need the web app to be up, so its row comes before the check for it.
        OpenScreen(stringResource(R.string.settings_pair_qr), example.hq.spike.ui.pairing.PairingActivity::class.java)
        val d = ui.snap.devices
        if (Unanswered(ui, "devices", d != null) || d == null) return@Page
        val shape = RoundedCornerShape(12.dp)
        val live = code != null && left > 0
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Hairline, shape).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Text(stringResource(R.string.settings_sign_in_another_device), style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.fillMaxWidth())
            if (live && code != null) {
                Text(SettingsWords.groupCode(code.code), style = HqType.H2.copy(fontFamily = FontFamily.Monospace, fontSize = 28.sp, lineHeight = 36.sp, letterSpacing = 0.06.em), color = Mts.Ink)
                Text(SettingsWords.codeHelp(example.hq.spike.ui.shell.rememberWords(), SettingsWords.clock(left)), style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.Center)
                TextLink(stringResource(R.string.settings_new_code), Mts.BrandBlue, enabled = ui.busy == null) { actions.onNewCode() }
            } else {
                Text(stringResource(if (code != null) R.string.settings_code_expired else R.string.settings_get_code_hint), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.fillMaxWidth())
                Row(Modifier.align(Alignment.Start).heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).clickable(enabled = ui.busy == null, role = Role.Button) { actions.onNewCode() }.padding(horizontal = 24.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    if (ui.busy == "code") Spinner(18)
                    Text(stringResource(R.string.settings_new_code), style = HqType.BodySmBold, color = Mts.Ink)
                }
            }
        }
        val list = SettingsWords.devices(example.hq.spike.ui.shell.rememberWords(), d)
        Label(stringResource(R.string.settings_label_signed_in))
        val moved = ui.notif.server.moved
        if (list.isEmpty() && moved) InfoCard(stringResource(R.string.moved_no_devices_title), stringResource(R.string.moved_no_devices_body))
        else if (list.isEmpty()) Text(stringResource(R.string.settings_no_other_device_is), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 4.dp, end = 16.dp))
        else {
            Group {
                list.forEachIndexed { i, dev ->
                    if (i > 0) Divider()
                    Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 16.dp, top = 8.dp, end = 4.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(if (dev.current) stringResource(R.string.settings_device_this_phone, dev.label) else dev.label, style = HqType.BodyBold, color = Mts.Ink)
                            Text(dev.line, style = HqType.BodySm, color = Mts.InkMuted)
                        }
                        if (!dev.current) TextLink(stringResource(R.string.device_sign_out), Mts.Ink, enabled = ui.busy == null) { actions.onAskSignOut(dev) }
                    }
                }
            }
        }
        if (moved && ui.notif.server.address != null && !ui.notif.server.connected) {
            Label(stringResource(R.string.settings_server_hq_title))
            InfoCard(stringResource(R.string.moved_server_signed_out_title), stringResource(R.string.moved_server_signed_out_body))
        }
        Box(Modifier.padding(start = 4.dp, top = 4.dp)) { TextLink(stringResource(R.string.sign_out_all_act), Mts.Ink, enabled = ui.busy == null && !ui.restarting) { actions.onSignOutAll(true) } }
    }
}

// ---------------------------------------------------------------- spending limits

@Composable
private fun Spending(ui: SettingsUi, actions: SettingsActions) {
    val w = example.hq.spike.ui.shell.rememberWords()
    Page {
        val b = ui.snap.budgets
        if (Unanswered(ui, "budgets", b != null) || b == null) { MobileData(ui, actions); return@Page }
        val cards = SettingsWords.budgetCards(w, b, ui.now)
        Label(stringResource(R.string.settings_label_budgets))
        if (cards.isEmpty()) Notice(SettingsWords.noBudgets(w, b), HqIcons.Info)
        else Group { cards.forEachIndexed { i, c -> if (i > 0) Divider(); BudgetCardView(c) } }
        val forecast = SettingsWords.forecast(w, ui.snap.forecast)
        when {
            forecast == null && "forecast" in ui.snap.unavailable -> { Label(stringResource(R.string.settings_label_forecast)); Text(stringResource(R.string.settings_the_forecast_is_not), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp)) }
            forecast?.note != null -> { Label(stringResource(R.string.settings_label_forecast)); Text(forecast.note, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp)) }
            forecast != null -> { Label(stringResource(R.string.settings_label_forecast)); Facts(forecast.rows) }
        }
        val services = SettingsWords.serviceRows(w, ui.snap)
        if (services.isNotEmpty()) {
            Label(stringResource(R.string.settings_label_by_service))
            Facts(services)
        }
        MobileData(ui, actions)
        val openWeb = LocalOpenWeb.current
        Text(stringResource(R.string.settings_changing_a_limit_is), style = SmallText, color = Mts.InkMuted, modifier = Modifier.padding(start = 20.dp, top = 12.dp, end = 20.dp))
        TextLink(stringResource(R.string.settings_open_usage_page), Mts.BrandBlue) { openWeb(WebPage.Usage) }
    }
}

@Composable
private fun BudgetCardView(c: SettingsWords.BudgetCard) {
    val fill = when (c.tone) { SettingsWords.BudgetTone.Ok -> Mts.BrandGreen; SettingsWords.BudgetTone.Warning -> Mts.Amber; SettingsWords.BudgetTone.Exceeded -> Mts.DangerText }
    Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Bottom) {
            Text(c.title, style = HqType.BodyBold, color = Mts.Ink)
            Text(c.covers, style = SmallText, color = Mts.InkMuted, modifier = Modifier.weight(1f))
        }
        Text(c.used, style = HqType.BodySm, color = Mts.Ink)
        Box(
            Modifier.fillMaxWidth().height(BAR_HEIGHT).clip(RoundedCornerShape(BAR_HEIGHT / 2)).background(Mts.Surface200)
                .semantics { progressBarRangeInfo = ProgressBarRangeInfo(c.fraction, 0f..1f) },
        ) { Box(Modifier.fillMaxWidth(c.fraction).fillMaxHeight().background(fill)) }
        Text(c.stand, style = SmallText, color = Mts.InkMuted)
        c.resetNote?.let { Text(it, style = SmallText, color = Mts.InkMuted) }
    }
}

// ---------------------------------------------------------------- background work

@Composable
internal fun Background(ui: SettingsUi, actions: SettingsActions) {
    val active = ui.mode == "active"
    val scroller = rememberSectionScroller(ui, actions.onSectionConsumed)
    Page(scroller) {
        Column(Modifier.padding(start = 16.dp, top = 8.dp, end = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ModeCard(stringResource(R.string.settings_mode_active_title), stringResource(R.string.settings_mode_active_body), active, !ui.restarting) { actions.onMode(true) }
            ModeCard(stringResource(R.string.about_background_off), stringResource(R.string.settings_mode_standby_body), !active, !ui.restarting) { actions.onMode(false) }
        }
        Box(Modifier.padding(top = 12.dp)) { Notice(stringResource(R.string.settings_one_device_only), HqIcons.Warning, warn = true) }
        Label(stringResource(R.string.assist_group_right_now), Modifier.sectionTarget(scroller, ScrollTarget.BackgroundRightNow))
        Facts(SettingsWords.rightNow(example.hq.spike.ui.shell.rememberWords(), ui.snap, active, ui.restarting))
        val keep = PhoneWords.keepRunning(example.hq.spike.ui.shell.rememberWords(), ui.keep, ui.now)
        Label(stringResource(R.string.settings_keep_running_section), Modifier.sectionTarget(scroller, ScrollTarget.BackgroundKeepRunning))
        Facts(listOf(FactRow(stringResource(R.string.settings_page_keep_running), PhoneWords.keepSummary(example.hq.spike.ui.shell.rememberWords(), ui.keep, ui.now), keep.noticeTitle, opens = SettingsPage.KeepRunning)), actions.onPage)
    }
}

/** One of the two run modes as a radio card: the whole card is the tap target, and the selected one has a green border. */
@Composable
private fun ModeCard(title: String, text: String, selected: Boolean, enabled: Boolean, onClick: () -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.fillMaxWidth().alpha(if (enabled) 1f else DISABLED_ALPHA).clip(shape).background(Mts.Surface100).border(1.dp, if (selected) Mts.BrandGreen else Mts.Line, shape)
            .selectable(selected = selected, enabled = enabled, role = Role.RadioButton, onClick = onClick).padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top,
    ) {
        Box(Modifier.padding(top = 1.dp).size(22.dp).border(2.dp, if (selected) Mts.BrandGreen else Mts.Line, CircleShape), contentAlignment = Alignment.Center) {
            if (selected) Box(Modifier.size(13.dp).background(Mts.BrandGreen, CircleShape))
        }
        Column {
            Text(title, style = HqType.BodyBold, color = Mts.Ink)
            Text(text, style = HqType.BodySm, color = Mts.InkMuted)
        }
    }
}

@Composable
internal fun DialogButton(label: String, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 24.dp), contentAlignment = Alignment.Center) {
        Text(label, style = HqType.BodySmBold, color = Mts.Ink)
    }
}

// ---------------------------------------------------------------- keep running and notifications

/** A card with a bold line over a plain one, amber-bordered when something needs a look. */
@Composable
internal fun NoticeCard(title: String, text: String, warn: Boolean) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, if (warn) Mts.Amber else Mts.Hairline, shape).padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top,
    ) {
        Icon(if (warn) HqIcons.Warning else HqIcons.Check, null, Modifier.padding(top = 1.dp).size(24.dp), tint = if (warn) Mts.Amber else Mts.BrandGreen)
        Column {
            Text(title, style = HqType.BodyBold, color = Mts.Ink)
            Text(text, style = HqType.BodySm, color = Mts.Ink)
        }
    }
}

@Composable
private fun KeepRunning(ui: SettingsUi, actions: SettingsActions) {
    val page = PhoneWords.keepRunning(example.hq.spike.ui.shell.rememberWords(), ui.keep, ui.now)
    Page {
        Box(Modifier.padding(top = 4.dp)) { NoticeCard(page.noticeTitle, page.notice, page.warn) }
        Label(stringResource(R.string.settings_label_checked))
        Group {
            page.checked.forEachIndexed { i, r ->
                if (i > 0) Divider()
                val tap = r.fix?.let { fix -> Modifier.clickable(role = Role.Button) { actions.onFix(fix) } } ?: Modifier
                Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).then(tap).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(r.title, style = HqType.BodyBold, color = Mts.Ink)
                        Text(r.value, style = HqType.BodySm, color = if (r.ok) Mts.InkMuted else Mts.Amber)
                    }
                    if (r.ok) Icon(HqIcons.Check, null, Modifier.size(22.dp), tint = Mts.BrandGreen) else Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
                }
            }
        }
        if (page.sleepingApps) {
            Label(stringResource(R.string.settings_label_cannot_check))
            Group {
                Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable(role = Role.Button) { actions.onFix(PhoneFix.SleepingApps) }.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(HqIcons.Info, null, Modifier.size(24.dp), tint = Mts.InkMuted)
                    Column(Modifier.weight(1f)) {
                        Text(stringResource(R.string.settings_sleeping_apps), style = HqType.BodyBold, color = Mts.Ink)
                        Text(stringResource(R.string.settings_samsung_may_put_agenthq), style = HqType.BodySm, color = Mts.InkMuted)
                    }
                    Icon(HqIcons.External, null, Modifier.size(20.dp), tint = Mts.InkMuted)
                }
            }
        }
        Label(stringResource(R.string.settings_label_history))
        Facts(listOf(page.history))
    }
}

// ---------------------------------------------------------------- add a key

/** The one place a key is typed: a sheet over Settings. The key lives only in this field, is shown as dots unless revealed, and goes away with the sheet. */
@Composable
private fun KeySheetView(sheet: KeySheet, actions: SettingsActions) {
    var typed by remember { androidx.compose.runtime.mutableStateOf("") }
    var reveal by remember { androidx.compose.runtime.mutableStateOf(false) }
    val words = SettingsWords.keyWords(example.hq.spike.ui.shell.rememberWords(), sheet.name, sheet.replace)
    val context = androidx.compose.ui.platform.LocalContext.current
    val shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
    val field = RoundedCornerShape(12.dp)
    val bad = sheet.error != null
    BackHandler(enabled = !sheet.checking, onBack = actions.onKeyClose)
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = SCRIM_ALPHA)).clickable(enabled = !sheet.checking, interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = actions.onKeyClose))
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().clip(shape).background(Mts.Surface100).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {}
                .imePadding().navigationBarsPadding().padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(Modifier.align(Alignment.CenterHorizontally).size(32.dp, 4.dp).clip(RoundedCornerShape(2.dp)).background(Mts.Line))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(words.title, style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                IconAction(HqIcons.Close, stringResource(R.string.close), iconSize = 24.dp, onClick = actions.onKeyClose)
            }
            Text(words.body, style = HqType.BodySm, color = Mts.InkMuted)
            Row(
                Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(field).background(Mts.Surface000).border(2.dp, if (bad) Mts.DangerText else Mts.Line, field).padding(start = 16.dp, top = 8.dp, end = 8.dp, bottom = 8.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_your_key), style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = if (bad) Mts.DangerText else Mts.InkMuted)
                    androidx.compose.foundation.text.BasicTextField(
                        typed, { typed = it }, Modifier.fillMaxWidth(), enabled = !sheet.checking, singleLine = true,
                        textStyle = HqType.Body.copy(color = Mts.Ink, fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace, letterSpacing = 0.1.em),
                        cursorBrush = androidx.compose.ui.graphics.SolidColor(Mts.Ink),
                        visualTransformation = if (reveal) androidx.compose.ui.text.input.VisualTransformation.None else androidx.compose.ui.text.input.PasswordVisualTransformation(),
                        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.Password, autoCorrectEnabled = false),
                    )
                }
                IconAction(if (reveal) HqIcons.EyeOff else HqIcons.Eye, stringResource(if (reveal) R.string.settings_hide_key else R.string.settings_show_key), iconSize = 22.dp) { reveal = !reveal }
            }
            sheet.error?.let { Text(it, style = HqType.BodySm, color = Mts.DangerText, modifier = Modifier.padding(start = 16.dp, top = 0.dp)) }
            SettingsWords.keyHelpUrl(sheet.id)?.let { url ->
                Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button) { runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url)).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)) } }.padding(horizontal = 12.dp), contentAlignment = Alignment.CenterStart) {
                    Text(stringResource(R.string.settings_where_do_i_find), style = HqType.BodySmBold, color = Mts.BrandBlue)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                val ready = typed.isNotBlank() && !sheet.checking
                Row(
                    Modifier.weight(1f).heightIn(min = 48.dp).clip(CircleShape).background(if (ready || sheet.checking) Mts.BrandGreen else Mts.Surface200).clickable(enabled = ready, role = Role.Button) { actions.onKeySave(typed) },
                    horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (sheet.checking) Spinner(18, Mts.OnGreen)
                    Text(stringResource(if (sheet.checking) R.string.settings_checking_dots else R.string.settings_check_save), style = HqType.BodySmBold, color = if (ready || sheet.checking) Mts.OnGreen else Mts.InkMuted)
                }
                Box(Modifier.heightIn(min = 48.dp).clickable(enabled = !sheet.checking, role = Role.Button, onClick = actions.onKeyClose).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.pairing_cancel), style = HqType.BodySmBold, color = Mts.InkMuted)
                }
            }
            Text(SettingsWords.keyNote(example.hq.spike.ui.shell.rememberWords()), style = SmallText, color = Mts.InkMuted)
        }
    }
}

// ---------------------------------------------------------------- capture and widgets

@Composable
private fun CaptureWidgets(ui: SettingsUi, actions: SettingsActions) {
    val c = ui.capture
    Page {
        Label(stringResource(R.string.settings_label_share_opens))
        Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(Triple("note", stringResource(R.string.settings_capture_note), "Notebooks/Inbox"), Triple("task", stringResource(R.string.tasks_task), "Personal \u00b7 Inbox"), Triple("last", stringResource(R.string.settings_share_last), "")).forEach { (key, title, end) ->
                val on = c.shareOpens == key
                val shape = RoundedCornerShape(12.dp)
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 50.dp).clip(shape).background(Mts.Surface100).border(1.dp, if (on) Mts.BrandGreen else Mts.Line, shape).selectable(selected = on, role = Role.RadioButton) { actions.onShareOpens(key) }.padding(horizontal = 16.dp, vertical = 12.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(Modifier.size(22.dp).border(2.dp, if (on) Mts.BrandGreen else Mts.Line, CircleShape), contentAlignment = Alignment.Center) { if (on) Box(Modifier.size(13.dp).background(Mts.BrandGreen, CircleShape)) }
                    Text(title, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
                    if (end.isNotEmpty()) Text(end, style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
        }
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_summarize_links), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.settings_summarize_links_note), style = HqType.BodySm, color = Mts.InkMuted)
                }
                Switch(checked = c.smartLinks, onCheckedChange = actions.onSmartLinks, colors = SwitchDefaults.colors(checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen, checkedBorderColor = Mts.BrandGreen))
            }
        }
        Label(stringResource(R.string.settings_label_widgets))
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.settings_show_titles_in_needs), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.settings_off_the_widget_shows), style = HqType.BodySm, color = Mts.InkMuted)
                }
                Switch(
                    checked = c.showTitles, onCheckedChange = actions.onShowTitles,
                    colors = SwitchDefaults.colors(checkedTrackColor = Mts.BrandGreen, checkedThumbColor = Mts.OnGreen, checkedBorderColor = Mts.BrandGreen, uncheckedTrackColor = Mts.Surface200, uncheckedThumbColor = Mts.Line, uncheckedBorderColor = Mts.Line),
                )
            }
        }
        Group {
            listOf(stringResource(R.string.start_capture), stringResource(R.string.inbox_tab_needs), stringResource(R.string.place_tasks)).forEachIndexed { i, name ->
                if (i > 0) Divider()
                Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(name, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
                    Box(Modifier.heightIn(min = 40.dp).clip(CircleShape).background(Mts.Surface200).clickable(role = Role.Button) { actions.onAddWidget(i) }.padding(horizontal = 16.dp), contentAlignment = Alignment.Center) { Text(stringResource(R.string.settings_add), style = HqType.BodySmBold, color = Mts.Ink) }
                }
            }
        }
        if (c.waiting.any { it.id !in ui.hiddenCaptures }) {
            Label(stringResource(R.string.outbox_heading))
            Group {
                c.waiting.filter { it.id !in ui.hiddenCaptures }.forEachIndexed { i, q ->
                    if (i > 0) Divider()
                    Column(Modifier.fillMaxWidth().padding(start = 16.dp, top = 12.dp, end = 8.dp, bottom = 4.dp)) {
                        Text(q.title.ifBlank { stringResource(R.string.settings_untitled) }, style = HqType.BodyBold, color = Mts.Ink)
                        Text(PhoneWords.queuedLine(example.hq.spike.ui.shell.rememberWords(), q), style = HqType.BodySm, color = if (q.failure != null) Mts.Amber else Mts.InkMuted)
                        Row(horizontalArrangement = Arrangement.spacedBy(0.dp)) {
                            DialogButton(stringResource(R.string.outbox_send_now)) { actions.onSendNow(q.id) }
                            DialogButton(stringResource(R.string.settings_discard)) { actions.onDiscard(q.id) }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- about

@Composable
private fun About(ui: SettingsUi, actions: SettingsActions) {
    Page {
        BackupCard(ui, actions)
        ColdStorageCard()
        MovingCard()
        StorageGroup(ui, actions)
    }
}

// ---------------------------------------------------------------- technical details and what HQ noticed

internal val NARROW_DP = 360
internal val CELL_WIDTH = 72.dp

@Composable
private fun Technical(ui: SettingsUi, actions: SettingsActions) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val counters = remember { Counters.get(context) }
    val info = remember { context.packageManager.getPackageInfo(context.packageName, 0) }
    val code = if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.P) info.longVersionCode else @Suppress("DEPRECATION") info.versionCode.toLong()
    val version = info.versionName.orEmpty().ifBlank { "dev" }
    val hqVersion = ui.snap.health?.optString("version")?.takeIf { it.isNotBlank() }
    val hq = if (ui.snap.health != null) stringResource(R.string.state_connected) else stringResource(R.string.technical_hq_not_answering)
    var sheet by remember { mutableStateOf<String?>(null) }
    val w = example.hq.spike.ui.shell.rememberWords()
    val facts = SettingsWords.DetailFacts(version, code.toString(), android.os.Build.VERSION.RELEASE.orEmpty(), ui.mode == "active", hq)
    Page {
        Facts(
            listOf(
                FactRow(stringResource(R.string.technical_version), hqVersion ?: stringResource(R.string.technical_hq_not_answering)),
                FactRow(stringResource(R.string.technical_build), code.toString()),
                FactRow(stringResource(R.string.technical_android), facts.android),
                FactRow(stringResource(R.string.technical_hq), hq),
            ) + listOfNotNull(SettingsWords.notesCount(ui.snap)?.let { FactRow(stringResource(R.string.place_notes), it) }),
        )
        Group {
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { actions.onPage(SettingsPage.Noticed) }.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.noticed_title), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.noticed_sub), style = HqType.BodySm, color = Mts.InkMuted)
                }
                Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
            Divider()
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { sheet = SettingsWords.copyDetails(w, facts, counters.snapshot()) }.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(HqIcons.Copy, null, Modifier.size(22.dp), tint = Mts.Ink)
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.copy_details_title), style = HqType.BodyBold, color = Mts.Ink)
                    Text(stringResource(R.string.copy_details_sub), style = HqType.BodySm, color = Mts.InkMuted)
                }
            }
            Divider()
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(enabled = !ui.restarting, role = Role.Button) { actions.onRestart(true) }.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(if (ui.restarting) R.string.settings_restarting_hq_dots else R.string.restart_act), style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
                if (ui.restarting) Spinner(20) else Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
            }
        }
        Text(stringResource(R.string.technical_note), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp))
    }
    sheet?.let { CopyDetailsSheet(it) { sheet = null } }
}

/** Shows the exact text before it goes on the clipboard; Copy puts that text there and nothing else. */
@Composable
internal fun CopyDetailsSheet(text: String, onClose: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val copied = stringResource(R.string.copy_details_copied)
    val attention = example.hq.spike.ui.shell.LocalAttention.current
    BackHandler(onBack = onClose)
    Sheet(onClose) {
        Text(stringResource(R.string.copy_details_title), style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(bottom = 8.dp))
        Text(stringResource(R.string.copy_details_body), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(bottom = 12.dp))
        Box(Modifier.weight(1f, fill = false).fillMaxWidth().clip(RoundedCornerShape(8.dp)).background(Mts.Surface100).verticalScroll(rememberScrollState()).padding(12.dp)) {
            Text(text, style = SmallText, fontFamily = FontFamily.Monospace, color = Mts.Ink)
        }
        Row(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
            DialogButton(stringResource(R.string.copy_details_not_now), onClose)
            DialogButton(stringResource(R.string.copy_details_copy)) {
                context.getSystemService(android.content.ClipboardManager::class.java).setPrimaryClip(android.content.ClipData.newPlainText("HQ details", text))
                attention?.say(copied)
                onClose()
            }
        }
    }
}

/** A card at the foot of the page over a scrim; a tap outside closes it. */
@Composable
internal fun Sheet(onClose: () -> Unit, content: @Composable androidx.compose.foundation.layout.ColumnScope.() -> Unit) {
    Box(Modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = SCRIM_ALPHA)).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClose))
        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().heightIn(max = 560.dp).clip(RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)).background(Mts.Surface200).navigationBarsPadding().padding(24.dp), content = content)
    }
}

@Composable
internal fun CardFrame(shape: androidx.compose.ui.graphics.Shape, warn: Boolean, content: @Composable () -> Unit) {
    Column(
        Modifier.padding(start = 16.dp, top = 4.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, if (warn) Mts.Amber else Mts.Line, shape).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) { content() }
}

/** The title is ink either way: a warning gets an amber mark, a good card a grey check. */
@Composable
internal fun CardTitle(text: String, warn: Boolean) {
    Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(if (warn) HqIcons.Warning else HqIcons.Check, null, Modifier.size(22.dp), tint = if (warn) Mts.Amber else Mts.InkMuted)
        Text(text, style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.semantics { heading() })
    }
}

/** Full width, so it stays whole at the largest text size. Green when [filled], outlined otherwise. */
@Composable
internal fun CardButton(label: String, filled: Boolean, onClick: () -> Unit) {
    Box(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(CircleShape).then(if (filled) Modifier.background(Mts.BrandGreen, CircleShape) else Modifier.border(1.dp, Mts.Line, CircleShape))
            .clickable(role = Role.Button, onClick = onClick).padding(horizontal = 24.dp, vertical = 8.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = if (filled) Mts.OnGreen else Mts.Ink, textAlign = androidx.compose.ui.text.style.TextAlign.Center) }
}

/** What the old phone should know before a move: only notes travel, and the copy button waits for the safe-mode .zip. */
@Composable
private fun MovingCard() {
    val shape = RoundedCornerShape(12.dp)
    Column(
        Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(stringResource(R.string.old_phone_title), style = HqType.BodyBold, color = Mts.Ink)
        Text(stringResource(R.string.old_phone_body), style = HqType.BodySm, color = Mts.Ink)
        Text(stringResource(R.string.old_phone_older), style = HqType.BodySm, color = Mts.Ink)
        Row(
            Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).padding(horizontal = 24.dp).semantics { disabled() },
            verticalAlignment = Alignment.CenterVertically,
        ) { Text(stringResource(R.string.old_phone_save_copy), style = HqType.BodySmBold, color = Mts.InkMuted) }
        Text(stringResource(R.string.old_phone_note), style = HqType.BodySm, color = Mts.InkMuted)
    }
}

/** A read-only list pushed from a Safety row: the standing notice, then its rows. */
@Composable
private fun ReadOnlyList(ui: SettingsUi, empty: String, rows: @Composable () -> Unit) {
    Page {
        Notice(stringResource(R.string.settings_read_only_notice))
        if (ui.snap.settings == null) Unanswered(ui, "settings", false)
        else Box(Modifier.padding(top = 12.dp)) { Column { rows() } }
    }
}

/** An address or other literal text, as it is written. */
@Composable
private fun MonoRow(text: String) {
    Group { Text(text, style = HqType.BodySm.copy(fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace), color = Mts.Ink, modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 16.dp)) }
}

/** The server HQ's own row, only while a server is kept: its name and whether it is signed in. The name is its host, as there is no name field. */
private fun serverRow(w: example.hq.spike.ui.shell.Words, c: android.content.Context): HomeRow? {
    val host = example.hq.spike.ServerHq.address(c)?.substringAfter("://")?.substringBefore('/') ?: return null
    return SettingsWords.serverRow(w, host, example.hq.spike.ServerHq.signedOut(c))
}
