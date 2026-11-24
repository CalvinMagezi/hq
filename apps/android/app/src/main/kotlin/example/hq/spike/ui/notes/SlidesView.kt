package example.hq.spike.ui.notes

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.BlockRow
import example.hq.spike.data.CardRow
import example.hq.spike.data.HqServices
import example.hq.spike.data.SlideBodies
import example.hq.spike.data.suiteBlocks
import example.hq.spike.data.suiteCards
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** The deck as the phone reads it: the cards hq lists, and the note's blocks the cards' words come from. */
data class DeckData(val cards: List<CardRow>, val blocks: List<BlockRow>)

/** Which card the editor is open on: a card of the deck, or a new one at the end. */
private sealed interface CardTarget {
    data class Existing(val index: Int) : CardTarget
    data object New : CardTarget
}

private const val PREVIEW_LINES = 3
private const val SLIDE_RATIO = 16f / 9f
private const val OPAQUE = 0xFF000000L

/** `#RRGGBB` as a colour, or null when it is not one. */
internal fun hexColor(text: String?): Color? {
    val digits = text?.removePrefix("#")?.takeIf { it.length == 6 } ?: return null
    return digits.toLongOrNull(16)?.let { Color(OPAQUE or it) }
}

/**
 * A slides note as its cards: each one a 16:9 tile with its title and first lines. Tapping one opens the card editor. [version]
 * is the note's revision, so the cards are asked for again when the note was written by someone else. [onHold] says whether
 * the editor is open, so that change waits until it closes.
 */
@Composable
fun SlidesView(path: String, version: String?, onNotice: (String) -> Unit, onHold: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val client = HqServices.get(LocalContext.current).client
    var tick by remember(path) { mutableIntStateOf(0) }
    val deck by produceState<Result<DeckData>?>(null, path, version, tick) {
        value = runCatching { DeckData(client.suiteCards(path), client.suiteBlocks(path)) }
    }
    var target by remember(path) { mutableStateOf<CardTarget?>(null) }
    val data = deck?.getOrNull()
    Column(modifier) {
        when {
            deck == null -> Text(stringResource(R.string.slides_loading), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
            data == null -> Text(stringResource(R.string.slides_load_failed), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
            else -> CardList(data, onOpen = { target = CardTarget.Existing(it) }, onAdd = { target = CardTarget.New })
        }
    }
    val open = target
    if (open != null && data != null) {
        HoldReload(onHold)
        val card = (open as? CardTarget.Existing)?.let { data.cards.getOrNull(it.index) }
        CardEditor(path, card, data, onClose = { changed -> target = null; if (changed) tick++ }, onNotice = onNotice)
    }
}

@Composable
internal fun CardList(data: DeckData, onOpen: (Int) -> Unit, onAdd: () -> Unit) {
    LazyColumn(Modifier.fillMaxWidth(), contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (data.cards.isEmpty()) item { Text(stringResource(R.string.slides_empty), style = HqType.Body, color = Mts.Ink) }
        items(data.cards, key = { it.index to it.hash }) { card -> CardTile(card, SlideBodies.preview(SlideBodies.bodyOf(data.blocks, card.index), PREVIEW_LINES)) { onOpen(card.index) } }
        item {
            TextButton(onAdd, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.slides_add), style = HqType.BodyBold, color = Mts.BrandBlue) }
        }
    }
}

/** One card at 16:9, in the colours the card itself asks for. */
@Composable
private fun CardTile(card: CardRow, preview: String, onClick: () -> Unit) {
    val back = hexColor(card.background) ?: Mts.Surface100
    val ink = hexColor(card.color) ?: Mts.Ink
    val shape = RoundedCornerShape(12.dp)
    Box(
        Modifier.fillMaxWidth().aspectRatio(SLIDE_RATIO).clip(shape).background(back).border(1.dp, Mts.Line, shape)
            .clickable(role = Role.Button, onClickLabel = stringResource(R.string.slides_open_card), onClick = onClick).padding(16.dp),
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("${card.index + 1}", style = HqType.Caption, color = ink)
            Text(card.title, style = HqType.H2, color = ink, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (preview.isNotEmpty()) Text(preview, style = HqType.BodySm, color = ink, maxLines = PREVIEW_LINES, overflow = TextOverflow.Ellipsis)
        }
        if (card.notes.isNotEmpty()) {
            Text(stringResource(R.string.slides_has_notes), style = HqType.Caption, color = ink, modifier = Modifier.align(androidx.compose.ui.Alignment.BottomEnd))
        }
    }
}

/** Keeps the open note from being reloaded under a dialog: [hold] is told true while this is on screen and false once it leaves. */
@Composable
fun HoldReload(hold: (Boolean) -> Unit) {
    androidx.compose.runtime.DisposableEffect(Unit) {
        hold(true)
        onDispose { hold(false) }
    }
}

/** A row of choices, one picked. */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
internal fun <T> Choices(items: List<T>, picked: T, label: @Composable (T) -> String, onPick: (T) -> Unit) {
    androidx.compose.foundation.layout.FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        items.forEach { item ->
            val on = item == picked
            TextButton({ onPick(item) }, Modifier.heightIn(min = 48.dp)) {
                Text((if (on) "● " else "○ ") + label(item), style = if (on) HqType.BodySmBold else HqType.BodySm, color = if (on) Mts.BrandBlue else Mts.InkMuted)
            }
        }
    }
}
