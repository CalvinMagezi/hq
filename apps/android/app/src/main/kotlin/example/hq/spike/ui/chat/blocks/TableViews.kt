package example.hq.spike.ui.chat.blocks

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.zIndex
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val HeaderRowHeight = 36.dp
private val BodyRowHeight = 40.dp
private val MinColumn = 72.dp
private val MaxColumn = 200.dp
private val CellPad = 24.dp
private val PinnedColumn = 104.dp
private val ResultIcon = 14.dp
private val ResultGap = 4.dp
private val HeaderStyle = HqType.Caption.copy(fontWeight = FontWeight.Bold)
private val CellStyle = HqType.BodySm
private val CellPadV = 10.dp
private val CellPadH = 12.dp
private const val MAX_CELL_LINES = 4

/** How wide each column is: its widest cell plus padding, kept between 72 and 200 dp (the board does not say how it chooses). */
private fun columnWidths(t: Block.Table, tm: TextMeasurer, d: Density): List<Dp> = t.headers.indices.map { c ->
    val cells = t.rows.map { row ->
        val word = Results.of(row[c])
        if (word == null) tm.measure(Plain.cell(row[c]), CellStyle).size.width else tm.measure(Plain.strip(row[c]), CellStyle.copy(fontWeight = FontWeight.Bold)).size.width + with(d) { (ResultIcon + ResultGap).roundToPx() }
    }
    val widest = (listOf(tm.measure(Plain.cell(t.headers[c]), HeaderStyle).size.width) + cells).max()
    with(d) { (widest.toDp() + CellPad).coerceIn(MinColumn, MaxColumn) }
}

/**
 * Each body row is 40 dp unless a cell needs more: a long cell wraps (up to four lines) instead of being cut off, and the
 * whole row grows with it so the pinned column and the sliding ones stay level.
 */
private fun rowHeights(t: Block.Table, widths: List<Dp>, tm: TextMeasurer, d: Density): List<Dp> = t.rows.map { row ->
    val text = row.indices.maxOf { c ->
        val room = with(d) { (widths[c] - CellPadH * 2).roundToPx() }.coerceAtLeast(1)
        tm.measure(Plain.cell(row[c]), CellStyle, overflow = TextOverflow.Ellipsis, maxLines = MAX_CELL_LINES, constraints = androidx.compose.ui.unit.Constraints(maxWidth = room)).size.height
    }
    maxOf(BodyRowHeight, with(d) { text.toDp() } + CellPadV * 2)
}

/** A table narrower than its box stretches its last column to the box edge, so the heading shade and row lines reach it. */
internal fun filled(widths: List<Dp>, room: Dp): List<Dp> {
    val extra = room - widths.fold(0.dp) { a, w -> a + w }
    return if (extra > 0.dp) widths.dropLast(1) + (widths.last() + extra) else widths
}

/** With the first column pinned it is a fixed 104 dp; longer names are cut with an ellipsis. */
private fun pinned(widths: List<Dp>): List<Dp> = listOf(PinnedColumn) + widths.drop(1)

private fun Align.toAlignment() = when (this) { Align.Start -> Alignment.CenterStart; Align.End -> Alignment.CenterEnd; Align.Center -> Alignment.Center }

@Composable
private fun Cell(text: String, width: Dp, align: Align, header: Boolean, height: Dp = if (header) HeaderRowHeight else BodyRowHeight, maxLines: Int = if (header) 1 else MAX_CELL_LINES) {
    val result = if (header) null else Results.of(text)
    Box(Modifier.width(width).height(height).padding(horizontal = CellPadH), contentAlignment = align.toAlignment()) {
        if (result != null) {
            Row(horizontalArrangement = Arrangement.spacedBy(ResultGap), verticalAlignment = Alignment.CenterVertically) {
                Icon(result.icon, null, Modifier.size(ResultIcon), tint = result.color)
                Text(text, style = CellStyle.copy(fontWeight = FontWeight.Bold), color = result.color, maxLines = 1)
            }
        } else {
            Text(Plain.cell(text), style = if (header) HeaderStyle else CellStyle, color = if (header) Mts.InkMuted else Mts.Ink, maxLines = maxLines, overflow = TextOverflow.Ellipsis,
                textAlign = if (align == Align.End) TextAlign.End else TextAlign.Start)
        }
    }
}

/** Pass and fail are a word and an icon, never colour alone. Only these two words are treated this way. */
private object Results {
    class Look(val icon: androidx.compose.ui.graphics.vector.ImageVector, val color: Color)
    fun of(text: String): Look? = when (text.trim().lowercase()) {
        "passed" -> Look(HqIcons.CheckSmall, Mts.BrandGreen)
        "failed" -> Look(HqIcons.Warning, Mts.Amber)
        else -> null
    }
}

/**
 * The table itself. With [pin] the first column stays at the left while the rest slide under it; [vertical] puts the
 * body in a vertical scroll with the heading row held at the top. Both parts share [hs] so heading and rows move together.
 */
@Composable
private fun TableLayout(t: Block.Table, widths: List<Dp>, heights: List<Dp>, pin: Boolean, hs: ScrollState, vs: ScrollState?, modifier: Modifier = Modifier) {
    val firstScrolling = if (pin) 1 else 0
    @Composable fun column(c: Int) = Column {
        t.rows.forEachIndexed { r, row ->
            Box(Modifier.drawBehind { drawLine(Mts.Hairline, Offset(0f, 0f), Offset(size.width, 0f), 1.dp.toPx()) }) { Cell(row[c], widths[c], t.aligns[c], false, heights[r]) }
        }
    }
    Column(modifier) {
        Row(Modifier.background(Mts.Surface100)) {
            if (pin) Box(Modifier.zIndex(1f).background(Mts.Surface100).pinEdge()) { Cell(t.headers[0], widths[0], t.aligns[0], true) }
            Row(Modifier.weight(1f).horizontalScroll(hs)) { (firstScrolling until t.headers.size).forEach { Cell(t.headers[it], widths[it], t.aligns[it], true) } }
        }
        Row(if (vs != null) Modifier.weight(1f, fill = false).verticalScroll(vs) else Modifier) {
            if (pin) Box(Modifier.zIndex(1f).background(Mts.Surface000).pinEdge()) { column(0) }
            Row(Modifier.weight(1f).horizontalScroll(hs)) { (firstScrolling until t.headers.size).forEach { column(it) } }
        }
    }
}

/** The line and soft shadow on the right edge of the pinned column, so the rest looks like it slides under it. */
private fun Modifier.pinEdge(): Modifier = drawBehind {
    drawLine(Mts.Line, Offset(size.width, 0f), Offset(size.width, size.height), 1.dp.toPx())
    drawRect(
        Brush.horizontalGradient(listOf(Color.Black.copy(alpha = 0.6f), Color.Transparent), startX = size.width, endX = size.width + 6.dp.toPx()),
        topLeft = Offset(size.width, 0f), size = androidx.compose.ui.geometry.Size(6.dp.toPx(), size.height),
    )
}

/** The table in a reply. Narrower than the box: plain. Wider: a bar with the size and Expand, a pinned first column, a fade at the edge. */
@Composable
fun TableBlockView(t: Block.Table, onExpand: () -> Unit) {
    val tm = rememberTextMeasurer()
    val density = LocalDensity.current
    val natural = remember(t) { columnWidths(t, tm, density) }
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val room = maxWidth - 2.dp
        val fits = natural.fold(0.dp) { a, w -> a + w } <= room
        val widths = if (fits) natural.map { it * (room.value / natural.fold(0.dp) { a, w -> a + w }.value) } else filled(pinned(natural), room)
        val heights = remember(t, widths) { rowHeights(t, widths, tm, density) }
        val hs = rememberScrollState()
        BlockFrame {
            if (!fits) {
                BlockHeader(HqIcons.Table, "${t.rows.size} rows · ${t.headers.size} columns", HeaderLabel, Mts.InkMuted) { IconAction(HqIcons.Expand, "Expand table", onClick = onExpand) }
            }
            Box {
                TableLayout(t, widths, heights, pin = !fits, hs = hs, vs = null)
                if (!fits && hs.canScrollForward) RightFade()
            }
        }
    }
}

/** The table on its own page: heading row and first column stay put, the rest scrolls both ways. */
@Composable
fun TablePageBody(t: Block.Table, modifier: Modifier = Modifier) {
    val tm = rememberTextMeasurer()
    val density = LocalDensity.current
    val natural = remember(t) { pinned(columnWidths(t, tm, density)) }
    val hs = rememberScrollState()
    val vs = rememberScrollState()
    BoxWithConstraints(modifier) {
        val widths = filled(natural, maxWidth)
        val heights = remember(t, widths) { rowHeights(t, widths, tm, density) }
        Column {
            Box(Modifier.fillMaxWidth().height(1.dp).background(Mts.Hairline))
            TableLayout(t, widths, heights, pin = true, hs = hs, vs = vs, modifier = Modifier.weight(1f, fill = false))
            ScrollHint(hs)
        }
    }
}

@Composable
private fun ScrollHint(hs: ScrollState) {
    val total = hs.maxValue.coerceAtLeast(1)
    Box(Modifier.padding(start = 112.dp, top = 10.dp, end = 16.dp).fillMaxWidth().height(4.dp).background(Mts.Surface200, RoundedCornerShape(2.dp))) {
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            Box(Modifier.padding(start = maxWidth * 0.5f * (hs.value.toFloat() / total)).width(maxWidth * 0.5f).height(4.dp).background(Mts.Line, RoundedCornerShape(2.dp)))
        }
    }
    Text("Swipe sideways for more columns. The first column and the heading row stay in place.", style = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp), color = Mts.InkMuted,
        modifier = Modifier.padding(start = 16.dp, top = 28.dp, end = 16.dp))
}

/** Tab separated, so it pastes into a spreadsheet. */
fun Block.Table.toTsv(): String = (listOf(headers) + rows).joinToString("\n") { it.joinToString("\t") }

/** The numbers table on a chart page: first column, then one column per series (or value and share for a pie). */
fun ChartSpec.numbersTable(): Block.Table {
    if (type.isRound) {
        val values = series[0].values
        val total = values.sum()
        return Block.Table(listOf(xLabel.ifBlank { "Label" }, "Value", "Share"), listOf(Align.Start, Align.End, Align.End),
            labels.mapIndexed { i, l -> listOf(l, plainNumber(values[i]), "${Charts.percent(values[i], total)}%") })
    }
    return Block.Table(listOf(xLabel.ifBlank { "Label" }) + series.map { it.name }, listOf(Align.Start) + series.map { Align.End },
        labels.mapIndexed { i, l -> listOf(l) + series.map { plainNumber(it.values[i]) } })
}

private fun plainNumber(n: Double): String = Charts.plain(n)

/** The numbers under a chart: 120 dp for the labels, 104 dp for each value, in a rounded box that scrolls sideways when the series do not fit. */
@Composable
fun NumbersTable(t: Block.Table, modifier: Modifier = Modifier) {
    val widths = remember(t) { t.headers.indices.map { if (it == 0) 120.dp else 104.dp } }
    val shape = RoundedCornerShape(12.dp)
    BoxWithConstraints(modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        val room = maxWidth
        Column(Modifier.fillMaxWidth().clip(shape).border(1.dp, Mts.Hairline, shape).horizontalScroll(rememberScrollState())) {
            Row(Modifier.widthIn(min = room).background(Mts.Surface100)) { t.headers.indices.forEach { Cell(t.headers[it], widths[it], t.aligns[it], true) } }
            t.rows.forEach { row ->
                Row(Modifier.widthIn(min = room).drawBehind { drawLine(Mts.Hairline, Offset(0f, 0f), Offset(size.width, 0f), 1.dp.toPx()) }) {
                    row.indices.forEach { Cell(row[it], widths[it], t.aligns[it], false, maxLines = 1) }
                }
            }
        }
    }
}
