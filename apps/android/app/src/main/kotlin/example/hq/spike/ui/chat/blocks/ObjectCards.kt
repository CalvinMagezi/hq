package example.hq.spike.ui.chat.blocks

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val CardShape = RoundedCornerShape(12.dp)
private val TileShape = RoundedCornerShape(10.dp)
private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)
private const val FOLD_AFTER = 3

/** Where a note chip stands: ready, being fetched, or known to be gone. */
enum class ChipState { Ready, Opening, Missing }

/** A note or a task the reply made: a tile, a small label, the title and a line of detail. The whole card is the button. */
@Composable
private fun ObjectCard(icon: ImageVector, label: String, title: String, meta: String, description: String, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(CardShape).background(Mts.Surface000).border(1.dp, Mts.Hairline, CardShape)
            .clickable(role = Role.Button, onClick = onClick).semantics { contentDescription = description }
            .padding(start = 12.dp, top = 8.dp, end = 8.dp, bottom = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(40.dp).background(Mts.Surface200, TileShape), contentAlignment = Alignment.Center) {
            Icon(icon, null, Modifier.size(22.dp), tint = Mts.Ink)
        }
        Column(Modifier.weight(1f)) {
            Text(label.uppercase(), style = SmallText.copy(letterSpacing = 0.06.em), color = Mts.InkMuted, maxLines = 1)
            Text(title, style = HqType.ChipTitle, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(meta, style = SmallText, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

@Composable
fun NoteCardView(card: NoteCard, age: String, onClick: () -> Unit) {
    val meta = listOf(card.trail, age).filter { it.isNotBlank() }.joinToString(" · ")
    ObjectCard(HqIcons.Notes, "New note", card.title, meta, "New note, ${card.title}. Opens the note.", onClick)
}

@Composable
fun TaskCardView(card: TaskCard, onClick: () -> Unit) =
    ObjectCard(HqIcons.Tasks, "New task", card.title, card.meta, "New task, ${card.title}. Opens Tasks.", onClick)

/** "Read 2 notes" and a chip for each; after three the rest fold into "+N more", which opens them all. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ReadNotes(paths: List<String>, stateOf: (String) -> ChipState, onOpen: (String) -> Unit) {
    var all by remember(paths) { mutableStateOf(false) }
    val shown = if (all || paths.size <= FOLD_AFTER + 1) paths else paths.take(FOLD_AFTER)
    Column {
        Text(
            (if (paths.size == 1) "Read 1 note" else "Read ${paths.size} notes").uppercase(),
            style = HqType.Badge.copy(letterSpacing = 0.06.em), color = Mts.InkMuted,
        )
        FlowRow(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            shown.forEach { NoteChip(it, stateOf(it)) { onOpen(it) } }
            if (shown.size < paths.size) MoreChip(paths.size - shown.size) { all = true }
        }
    }
}

@Composable
private fun NoteChip(path: String, state: ChipState, onClick: () -> Unit) {
    val name = NoteRefs.title(path)
    val missing = state == ChipState.Missing
    val text = when (state) {
        ChipState.Ready -> name
        ChipState.Opening -> "Opening…"
        ChipState.Missing -> "$name · not found"
    }
    val dim = state != ChipState.Ready
    Row(
        Modifier.heightIn(min = 36.dp).clip(CircleShape).chipBorder(missing).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        if (missing) Icon(HqIcons.Warning, null, Modifier.size(16.dp), tint = Mts.Amber) else Icon(HqIcons.Notes, null, Modifier.size(16.dp), tint = if (dim) Mts.InkMuted else Mts.Ink)
        Text(text, style = HqType.ToolLine, color = if (dim) Mts.InkMuted else Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun MoreChip(count: Int, onClick: () -> Unit) {
    Row(
        Modifier.heightIn(min = 36.dp).clip(CircleShape).chipBorder(false).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) { Text("+$count more", style = HqType.ToolLine, color = Mts.InkMuted, maxLines = 1) }
}

/** A solid line, or a dashed one for a note that cannot be opened. */
private fun Modifier.chipBorder(dashed: Boolean): Modifier = if (!dashed) border(1.dp, Mts.Line, CircleShape) else drawBehind {
    val r = size.height / 2
    drawRoundRect(Mts.Line, cornerRadius = CornerRadius(r), style = Stroke(1.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(4.dp.toPx(), 3.dp.toPx()))))
}

/** A file that came with a message: its name, what kind it is and how big, and an Open chip. */
@Composable
fun FileCardView(file: Attachment, onOpen: () -> Unit, modifier: Modifier = Modifier) {
    Row(
        modifier.fillMaxWidth().heightIn(min = 64.dp).clip(CardShape).background(Mts.Surface000).border(1.dp, Mts.Hairline, CardShape)
            .padding(start = 12.dp, top = 8.dp, end = 8.dp, bottom = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(40.dp).background(Mts.Surface200, TileShape), contentAlignment = Alignment.Center) {
            Icon(HqIcons.Notes, null, Modifier.size(22.dp), tint = Mts.Ink)
        }
        Column(Modifier.weight(1f)) {
            Text(file.name, style = HqType.ChipTitle, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text("${file.kind} · ${file.sizeWords}", style = SmallText, color = Mts.InkMuted, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Box(
            Modifier.heightIn(min = 32.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClickLabel = "Open ${file.name}", onClick = onOpen).padding(horizontal = 12.dp),
            contentAlignment = Alignment.Center,
        ) { Text("Open", style = HqType.ToolLine, color = Mts.Ink, maxLines = 1) }
    }
}
