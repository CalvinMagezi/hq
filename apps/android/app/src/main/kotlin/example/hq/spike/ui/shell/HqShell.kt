package example.hq.spike.ui.shell

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.HqStatus
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** The five places on the bar, left to right (design page 3, Navigation board). */
enum class Place(val label: String, val icon: ImageVector?) {
    Notes("Notes", HqIcons.Notes),
    Tasks("Tasks", HqIcons.Tasks),
    Chat("Chat", null),
    Helpers("Helpers", HqIcons.Helpers),
    Inbox("Inbox", HqIcons.Inbox),
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
val LocalGoTo = staticCompositionLocalOf<(Place) -> Unit> { {} }

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun HqShell(
    startAt: Place = Place.Chat,
    inboxBadge: Int = 0,
    status: HqStatus = HqStatus.Live,
    onSearch: () -> Unit = {},
    onSettings: () -> Unit = {},
    overlay: @Composable () -> Unit = {},
    content: @Composable (Place) -> Unit,
) {
    var current by rememberSaveable { mutableStateOf(startAt) }
    BackHandler(enabled = current != Place.Chat) { current = Place.Chat }
    val (statusText, statusColor) = when (status) {
        HqStatus.Live -> "Live" to Mts.BrandGreen
        is HqStatus.Starting -> "Starting HQ \u00b7 ${status.seconds} s" to Mts.Amber
        is HqStatus.Saved -> "Saved \u00b7 ${status.age}" to Mts.Amber
    }
    CompositionLocalProvider(LocalGoTo provides { current = it }) {
        Box(Modifier.fillMaxSize().background(Mts.Surface000)) {
            Column(Modifier.fillMaxSize().statusBarsPadding()) {
                TopBar(title = current.label, status = statusText, statusColor = statusColor, onSearch = onSearch, onSettings = onSettings)
                Box(Modifier.weight(1f).fillMaxWidth().imePadding()) { content(current) }
                if (!WindowInsets.isImeVisible) BottomBar(current, inboxBadge) { current = it }
            }
            overlay()
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
fun TopBar(title: String, status: String, statusColor: Color, onSearch: () -> Unit, onSettings: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().height(TopBarHeight).background(Mts.Surface000).padding(start = 16.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f).height(TouchTarget), verticalArrangement = Arrangement.Center) {
            Text(title, style = HqType.ScreenTitle, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Box(Modifier.size(8.dp).background(statusColor, CircleShape))
                Text(status, style = HqType.Caption, color = Mts.InkMuted, maxLines = 1)
            }
        }
        IconButton(onSearch, Modifier.size(TouchTarget)) { Icon(HqIcons.Search, "Search", Modifier.size(24.dp), tint = Mts.Ink) }
        IconButton(onSettings, Modifier.size(TouchTarget)) { Icon(HqIcons.Gear, "Settings", Modifier.size(24.dp), tint = Mts.Ink) }
    }
}

@Composable
fun BottomBar(current: Place, inboxBadge: Int, onSelect: (Place) -> Unit) {
    val navInset = with(LocalDensity.current) { WindowInsets.navigationBars.getBottom(this).toDp() }
    Row(
        Modifier.fillMaxWidth().background(Mts.Surface100)
            .height(BarContentHeight + maxOf(BarMinBottom, navInset)).padding(bottom = maxOf(BarMinBottom, navInset)),
    ) {
        for (place in Place.entries) {
            val selected = place == current
            Column(
                Modifier.weight(1f).fillMaxHeight().selectable(selected, role = Role.Tab, onClick = { onSelect(place) }),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = if (place == Place.Chat) Arrangement.Top else Arrangement.Center,
            ) {
                if (place == Place.Chat) RaisedChat(selected) else PlaceIcon(place, selected, if (place == Place.Inbox) inboxBadge else 0)
                Box(Modifier.height(4.dp))
                Text(place.label, style = HqType.NavLabel, color = if (selected) Mts.Ink else Mts.InkMuted, maxLines = 1)
            }
        }
    }
}

@Composable
private fun PlaceIcon(place: Place, selected: Boolean, badge: Int) {
    Box(Modifier.size(width = 64.dp, height = 32.dp)) {
        Box(
            Modifier.fillMaxSize().clip(RoundedCornerShape(16.dp)).background(if (selected) Mts.Surface200 else Color.Transparent),
            contentAlignment = Alignment.Center,
        ) {
            Icon(place.icon!!, null, Modifier.size(24.dp), tint = if (selected) Mts.BrandGreen else Mts.InkMuted)
        }
        if (badge > 0) {
            // The design puts the badge 4 dp above the pill and 10 dp in from its right edge.
            Text(
                if (badge > 99) "99+" else badge.toString(), style = HqType.Badge, color = Mts.OnGreen, textAlign = TextAlign.Center,
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
