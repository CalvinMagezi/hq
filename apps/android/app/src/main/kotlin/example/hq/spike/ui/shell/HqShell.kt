package example.hq.spike.ui.shell

import androidx.activity.compose.BackHandler
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.testTag
import example.hq.spike.data.Connection
import example.hq.spike.data.ConnectionTone
import example.hq.spike.data.ConnectionWords
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/**
 * Every place the shell can show. Three are on the bar, left to right: Notes, Chat, Inbox. Tasks and Helpers are where HQ
 * and its agents work: they keep their pages and open from an Inbox item, a chat card, search, a link or Settings, never
 * from the bar.
 */
enum class Place(val label: String, val icon: ImageVector?, val onBar: Boolean = true) {
    Notes("Notes", HqIcons.Notes),
    Tasks("Tasks", HqIcons.Tasks, onBar = false),
    Chat("Chat", null),
    Helpers("Helpers", HqIcons.Helpers, onBar = false),
    Inbox("Inbox", HqIcons.Inbox),
    ;

    companion object {
        /** The bar's places, in order. */
        val bar: List<Place> get() = Place.entries.filter { it.onBar }
    }
}

private val BarContentHeight = 80.dp
private val BarMinBottom = 16.dp
private val TopBarHeight = 64.dp
private val TouchTarget = 48.dp
private val RaisedButton = 56.dp
private val RaisedLift = 10.dp

/**
 * The same two bars on every main screen. Each place keeps where the user was; Back from any other
 * place goes to Chat, and Back from Chat leaves the app. The bottom bar steps aside while the keyboard is up.
 */
/** Lets a screen or an overlay send the shell to another place, such as a note card going to Notes. */
/** Opens the one search where a pill, a magnifier or a missing note asks to start. */
val LocalOpenSearch = staticCompositionLocalOf<(example.hq.spike.ui.search.SearchStart) -> Unit> { {} }

val LocalGoTo = staticCompositionLocalOf<(Place) -> Unit> { {} }

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun HqShell(
    startAt: Place = Place.Chat,
    inboxBadge: Int = 0,
    status: Connection = Connection.Connected,
    /** The line under the bar while HQ is not Connected (the offline strip); null when there is none. */
    strip: @Composable (Place) -> Unit = {},
    onSearch: () -> Unit = {},
    onOpenSearch: (example.hq.spike.ui.search.SearchStart) -> Unit = {},
    onSettings: () -> Unit = {},
    /** The status line is one button that opens the Connection sheet. */
    onStatus: () -> Unit = {},
    /** What a place adds to the bar: a line in front of the status ("43 active") and buttons between search and settings. */
    subline: (Place) -> String? = { null },
    topActions: @Composable (Place) -> Unit = {},
    updateDot: Boolean = false,
    /** A quiet dot on the Notes place when HQ filed something new; data comes later. */
    notesDot: Boolean = false,
    attention: AttentionBudget = remember { AttentionBudget() },
    request: Place? = null,
    onRequestHandled: () -> Unit = {},
    aboveBar: @Composable BoxScope.(Place) -> Unit = {},
    overlay: @Composable () -> Unit = {},
    content: @Composable (Place) -> Unit,
) {
    var current by rememberSaveable { mutableStateOf(startAt) }
    LaunchedEffect(request) { if (request != null) { current = request; onRequestHandled() } }
    BackHandler(enabled = current != Place.Chat) { current = Place.Chat }
    val words = ConnectionWords.of(status)
    val statusText = stringResource(words.res, *words.args.toTypedArray())
    val statusColor = when (words.tone) { ConnectionTone.Quiet -> Mts.InkMuted; ConnectionTone.Working -> Mts.BrandBlue; ConnectionTone.Warn -> Mts.Amber }
    val bus = remember { ReselectBus() }
    CompositionLocalProvider(LocalGoTo provides { current = it }, LocalReselect provides bus.taps, LocalOpenSearch provides onOpenSearch, LocalAttention provides attention) {
        Box(Modifier.fillMaxSize().background(Mts.Surface000)) {
            Column(Modifier.fillMaxSize().statusBarsPadding()) {
                TopBar(title = current.label, status = subline(current)?.let { "$it \u00b7 $statusText" } ?: statusText, statusColor = statusColor, onSearch = onSearch, onSettings = onSettings, showSearch = current in SEARCH_PLACES, updateDot = updateDot, actions = { topActions(current) }, stopped = status is Connection.Stopped, onStatus = onStatus)
                strip(current)
                BannerSlot(attention)
                Box(Modifier.weight(1f).fillMaxWidth().imePadding()) { content(current); aboveBar(current) }
                if (!WindowInsets.isImeVisible) BottomBar(current, inboxBadge, notesDot) { if (it == current) bus.tap(it) else current = it }
            }
            overlay()
            // Above the overlays, so an Undo from a page that covers the bar still shows; lifted over the bar while it is there.
            val navInset = with(LocalDensity.current) { WindowInsets.navigationBars.getBottom(this).toDp() }
            val lift = if (WindowInsets.isImeVisible) 0.dp else BarContentHeight + maxOf(BarMinBottom, navInset)
            SnackbarSlot(attention, aboveBar = lift, belowBar = 0.dp, Modifier.align(Alignment.BottomCenter).imePadding())
        }
    }
}

@Composable
fun NotBuilt(place: Place) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text(
            "${place.label} is not built in the native app yet.",
            style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.Center, modifier = Modifier.padding(horizontal = 32.dp),
        )
    }
}

@Composable
fun TopBar(title: String, status: String, statusColor: Color, onSearch: () -> Unit, onSettings: () -> Unit, showSearch: Boolean = true, updateDot: Boolean = false, actions: @Composable () -> Unit = {}, stopped: Boolean = false, onStatus: () -> Unit = {}) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = TopBarHeight).background(Mts.Surface000).padding(start = 16.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val open = Modifier.weight(1f).heightIn(min = TouchTarget).clickable(role = Role.Button, onClickLabel = stringResource(R.string.conn_open_label), onClick = onStatus)
        TitleAndStatus(
            open,
            title = { Text(title, style = HqType.ScreenTitle, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            status = {
                // The word says what HQ is doing, so it is read out when it changes; the dot alone would say nothing.
                Row(Modifier.semantics { liveRegion = LiveRegionMode.Polite }, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (stopped) Icon(HqIcons.Stop, null, Modifier.size(12.dp), tint = statusColor) else Box(Modifier.size(8.dp).background(statusColor, CircleShape))
                    Text(status, style = HqType.Caption, color = if (stopped) statusColor else Mts.InkMuted)
                }
            },
        )
        // Search is on Notes and Chat only (owner decision: the other places have their own find or filter); the gear is everywhere.
        if (showSearch) IconButton(onSearch, Modifier.size(TouchTarget)) { Icon(HqIcons.Search, "Search", Modifier.size(24.dp), tint = Mts.Ink) }
        actions()
        val gearWords = stringResource(if (updateDot) R.string.upd_gear_dot else R.string.settings_title)
        IconButton(onSettings, Modifier.size(TouchTarget)) {
            Box(contentAlignment = Alignment.TopEnd) {
                Icon(HqIcons.Gear, gearWords, Modifier.size(24.dp), tint = Mts.Ink)
                if (updateDot) Box(Modifier.size(8.dp).background(Mts.BrandBlue, CircleShape).testTag("update-dot"))
            }
        }
    }
}

/** Search is on the places that hold things to look through: Notes and Chat. The gear is on every place. */
private val SEARCH_PLACES = setOf(Place.Notes, Place.Chat)

@Composable
fun BottomBar(current: Place, inboxBadge: Int, notesDot: Boolean = false, onSelect: (Place) -> Unit) {
    val navInset = with(LocalDensity.current) { WindowInsets.navigationBars.getBottom(this).toDp() }
    Row(
        Modifier.fillMaxWidth().background(Mts.Surface100)
            .height(BarContentHeight + maxOf(BarMinBottom, navInset)).padding(bottom = maxOf(BarMinBottom, navInset)),
    ) {
        for (place in Place.bar) {
            val selected = place == current
            Column(
                Modifier.weight(1f).fillMaxHeight().selectable(selected, role = Role.Tab, onClick = { onSelect(place) }),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = if (place == Place.Chat) Arrangement.Top else Arrangement.Center,
            ) {
                if (place == Place.Chat) RaisedChat(selected) else PlaceIcon(place, selected, if (place == Place.Inbox) inboxBadge else 0, place == Place.Notes && notesDot)
                Box(Modifier.height(4.dp))
                Text(place.label, style = HqType.NavLabel, color = if (selected) Mts.Ink else Mts.InkMuted, maxLines = 1)
            }
        }
    }
}

@Composable
private fun PlaceIcon(place: Place, selected: Boolean, badge: Int, dot: Boolean = false) {
    val notesDotWords = stringResource(R.string.notes_dot_new)
    Box(Modifier.size(width = 64.dp, height = 32.dp)) {
        Box(
            Modifier.fillMaxSize().clip(RoundedCornerShape(16.dp)).background(if (selected) Mts.Surface200 else Color.Transparent),
            contentAlignment = Alignment.Center,
        ) {
            Icon(place.icon!!, null, Modifier.size(24.dp), tint = if (selected) Mts.BrandGreen else Mts.InkMuted)
        }
        if (dot && badge == 0) Box(Modifier.align(Alignment.TopEnd).offset(x = (-14).dp, y = 2.dp).size(8.dp).background(Mts.BrandGreen, CircleShape).testTag("notes-dot").semantics { contentDescription = notesDotWords })
        if (badge > 0) {
            // The design puts the badge 4 dp above the pill and 10 dp in from its right edge.
            Text(
                Counts.navBadge(badge), style = HqType.Badge, color = Mts.OnGreen, textAlign = TextAlign.Center,
                modifier = Modifier.align(Alignment.TopEnd).offset(x = (-10).dp, y = (-4).dp)
                    .widthIn(min = 16.dp).heightIn(min = 16.dp).background(Mts.BrandGreen, RoundedCornerShape(8.dp))
                    .padding(horizontal = 4.dp),
            )
        }
    }
}

/** The HQ mark from the design, 56 dp, lifted 10 dp above the bar; a green ring in Chat. */
@Composable
private fun RaisedChat(selected: Boolean) {
    Box(
        Modifier.negativeTop(RaisedLift).size(RaisedButton)
            .drawBehind { drawCircle(Mts.Surface100, radius = size.minDimension / 2 + 3.dp.toPx()) }
            .clip(CircleShape).background(Mts.LauncherGround)
            .border(2.dp, if (selected) Mts.BrandGreen else Mts.Hairline, CircleShape),
    ) {
        Image(painterResource(R.drawable.hq_mark), null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
    }
}

/** Pulls a child up by [amount] without growing the space it takes, like a negative CSS margin-top. */
private fun Modifier.negativeTop(amount: Dp) = layout { measurable, constraints ->
    val p = measurable.measure(constraints)
    val up = amount.roundToPx()
    layout(p.width, p.height - up) { p.place(0, -up) }
}

private val TitleMinWidth = 72.dp
private val TitleStatusGap = 12.dp

/**
 * The title and its status word. The status has priority: it takes the width it needs and the title gets the rest, cut
 * with an ellipsis. When the title could not keep [TitleMinWidth] beside it, the status goes under the title instead (and
 * wraps if it must), at any text size, so the word that matters (Offline, Starting, Stopped) is never the one cut.
 */
@Composable
private fun TitleAndStatus(modifier: Modifier, title: @Composable () -> Unit, status: @Composable () -> Unit) {
    androidx.compose.ui.layout.Layout({ Box { title() }; Box { status() } }, modifier) { measurables, constraints ->
        val (titleM, statusM) = measurables
        val gap = TitleStatusGap.roundToPx()
        val max = constraints.maxWidth
        val statusNatural = statusM.maxIntrinsicWidth(Int.MAX_VALUE)
        val titleMin = minOf(titleM.maxIntrinsicWidth(Int.MAX_VALUE), TitleMinWidth.roundToPx())
        if (statusNatural + gap + titleMin <= max) {
            val s = statusM.measure(androidx.compose.ui.unit.Constraints(maxWidth = statusNatural))
            val t = titleM.measure(androidx.compose.ui.unit.Constraints(maxWidth = max - gap - s.width))
            val h = maxOf(t.height, s.height, constraints.minHeight)
            layout(max, h) { t.placeRelative(0, (h - t.height) / 2); s.placeRelative(t.width + gap, (h - s.height) / 2) }
        } else {
            val t = titleM.measure(androidx.compose.ui.unit.Constraints(maxWidth = max))
            val s = statusM.measure(androidx.compose.ui.unit.Constraints(maxWidth = max))
            val h = maxOf(t.height + s.height, constraints.minHeight)
            val top = (h - t.height - s.height) / 2
            layout(max, h) { t.placeRelative(0, top); s.placeRelative(0, top + t.height) }
        }
    }
}
