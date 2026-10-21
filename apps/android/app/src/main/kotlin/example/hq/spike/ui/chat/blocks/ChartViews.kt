package example.hq.spike.ui.chat.blocks

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Box
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.theme.ClearSans
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import example.hq.spike.ui.shell.HqIcons
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/** The fixed series colours, in order. A payload cannot choose them. Red and grey use MTS tokens (the board leaves them open). */
val SeriesPalette = listOf(Mts.BrandBlue, Mts.BrandGreen, Mts.Amber, Mts.Ink, Mts.DangerText, Mts.Line)
fun seriesColor(i: Int) = SeriesPalette[i % SeriesPalette.size]

private enum class Marker { Circle, Square, Diamond, Triangle }
private fun markerOf(series: Int) = Marker.entries[series % Marker.entries.size]

private val TickStyle = TextStyle(fontFamily = ClearSans, fontSize = 12.sp, fontWeight = FontWeight.Normal, color = Mts.InkMuted)
private val ValueStyle = TextStyle(fontFamily = ClearSans, fontSize = 12.sp, fontWeight = FontWeight.Bold, color = Mts.Ink)

private const val LEFT = 34f
private const val TOP = 16f
private const val BAR_MAX = 32f
private const val BAR_HEIGHT = 180
private const val LINE_HEIGHT = 210
const val PAGE_CHART_HEIGHT = 270

fun chartDescription(spec: ChartSpec): String =
    "${spec.type.title} chart" + spec.title.takeIf { it.isNotBlank() }?.let { ": " + it.replaceFirstChar { c -> c.lowercase() } }.orEmpty()

private fun plain(n: Double): String = if (n == kotlin.math.floor(n) && abs(n) < 1e15) n.toLong().toString() else Charts.round2(n).toString().trimEnd('0').trimEnd('.')

/** The card in a reply: header with Expand, legend, and the drawing. Tapping the chart also expands it. */
@Composable
fun ChartCard(spec: ChartSpec, onExpand: () -> Unit) {
    BlockFrame {
        BlockHeader(HqIcons.BarChart, spec.title.ifBlank { chartDescription(spec) }, HqType.BodySmBold, Mts.Ink) { IconAction(HqIcons.Expand, "Expand chart", onClick = onExpand) }
        if (spec.type.isRound) DonutBody(spec) else {
            Legend(spec, Modifier.padding(start = 12.dp, top = 10.dp, end = 12.dp))
            ChartCanvas(spec, height = if (spec.type == ChartType.Bar) BAR_HEIGHT else LINE_HEIGHT, selected = null, onSelect = null, modifier = Modifier.padding(top = if (spec.type == ChartType.Bar) 12.dp else 4.dp, bottom = 8.dp).clickableCanvas(onExpand))
        }
    }
}

private fun Modifier.clickableCanvas(onClick: () -> Unit) = pointerInput(Unit) { detectTapGestures { onClick() } }

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun Legend(spec: ChartSpec, modifier: Modifier = Modifier) {
    if (spec.type.isRound || spec.series.size < 2) return
    FlowRow(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        spec.series.forEachIndexed { i, s ->
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                LegendMarker(spec.type, i)
                Text(s.name, style = HqType.ToolLine, color = Mts.Ink)
            }
        }
    }
}

@Composable
private fun LegendMarker(type: ChartType, index: Int) {
    val color = seriesColor(index)
    Canvas(Modifier.size(14.dp)) {
        val c = Offset(size.width / 2, size.height / 2)
        if (type == ChartType.Bar) drawRoundRect(color, Offset(2.dp.toPx(), 2.dp.toPx()), Size(10.dp.toPx(), 10.dp.toPx()), CornerRadius(2.dp.toPx()))
        else marker(markerOf(index), c, 4.5.dp.toPx(), color, Color.Transparent, 2.5.dp.toPx())
    }
}

/** Line and bar drawing, sized from the board: plot inset 34 left, 16 top, 30 or 50 below depending on an x axis title. */
@Composable
fun ChartCanvas(spec: ChartSpec, height: Int, selected: Int?, onSelect: ((Int?) -> Unit)?, modifier: Modifier = Modifier) {
    val measurer = rememberTextMeasurer()
    val scale = remember(spec) { Charts.yScale(spec.series.flatMap { it.values }) }
    val description = chartDescription(spec)
    var input = modifier.fillMaxWidth().height(height.dp).semantics { contentDescription = description; role = Role.Image }
    if (onSelect != null) {
        input = input.pointerInput(spec) { detectTapGestures { p -> onSelect(indexAt(spec, size.width.toFloat(), density, p.x)) } }
            .pointerInput(spec) { detectDragGestures(onDragEnd = {}, onDrag = { change, _ -> onSelect(indexAt(spec, size.width.toFloat(), density, change.position.x)); change.consume() }) }
    }
    Canvas(input) { drawCartesian(spec, scale, measurer, selected) }
}

private fun plotLeft(spec: ChartSpec) = if (spec.yLabel.isNotBlank()) LEFT + 12f else LEFT
private fun plotRight(spec: ChartSpec) = if (spec.type == ChartType.Bar) 8f else 12f
private fun bottomInset(spec: ChartSpec) = if (spec.xLabel.isNotBlank()) 50f else 30f

private fun indexAt(spec: ChartSpec, widthPx: Float, density: Float, xPx: Float): Int {
    val n = spec.labels.size
    val left = plotLeft(spec) * density
    val right = widthPx - plotRight(spec) * density
    val x = xPx.coerceIn(left, right)
    return if (spec.type == ChartType.Bar) (((x - left) / (right - left)) * n).toInt().coerceIn(0, n - 1)
    else if (n == 1) 0 else (((x - left) / (right - left)) * (n - 1)).roundToInt().coerceIn(0, n - 1)
}

private fun DrawScope.drawCartesian(spec: ChartSpec, scale: Charts.YScale, tm: TextMeasurer, selected: Int?) {
    val left = plotLeft(spec).dp.toPx()
    val right = size.width - plotRight(spec).dp.toPx()
    val top = TOP.dp.toPx()
    val base = size.height - bottomInset(spec).dp.toPx()
    val plotH = base - top
    fun y(v: Double) = base - ((v - scale.min) / (scale.max - scale.min)).toFloat() * plotH
    val n = spec.labels.size
    val slot = (right - left) / n
    fun xBar(i: Int) = left + (i + 0.5f) * slot
    fun xLine(i: Int) = if (n == 1) (left + right) / 2 else left + i * (right - left) / (n - 1)
    val cx: (Int) -> Float = if (spec.type == ChartType.Bar) ::xBar else ::xLine

    scale.ticks.forEach { t ->
        drawLine(Mts.Hairline, Offset(left, y(t)), Offset(right, y(t)), 1.dp.toPx())
        text(tm, Charts.compact(t), TickStyle, left - 6.dp.toPx(), y(t) + 4.dp.toPx(), Align.End)
    }
    val every = Charts.labelEvery(n)
    val labelBase = base + (if (spec.type == ChartType.Bar) 18 else 22).dp.toPx()
    spec.labels.forEachIndexed { i, l -> if (i % every == 0) text(tm, Charts.clip(l), TickStyle, cx(i), labelBase, Align.Center) }
    if (spec.xLabel.isNotBlank()) text(tm, spec.xLabel, TickStyle, (left + right) / 2, size.height - 6.dp.toPx(), Align.Center)
    if (spec.yLabel.isNotBlank()) rotate(-90f, Offset(10.dp.toPx(), (top + base) / 2)) { text(tm, spec.yLabel, TickStyle, 10.dp.toPx(), (top + base) / 2, Align.Center) }

    if (spec.type == ChartType.Bar) drawBars(spec, tm, ::y, ::xBar, slot, base)
    else drawLines(spec, ::y, ::xLine)
    if (selected != null) drawSelection(spec, tm, selected, cx(selected), top, base, right)
}

private fun DrawScope.drawBars(spec: ChartSpec, tm: TextMeasurer, y: (Double) -> Float, cx: (Int) -> Float, slot: Float, base: Float) {
    val groupW = min(slot * 0.7f, BAR_MAX.dp.toPx() * min(spec.series.size, 2))
    val barW = groupW / spec.series.size
    val zero = y(0.0)
    val showValues = barW >= 20.dp.toPx()
    spec.series.forEachIndexed { si, s ->
        s.values.forEachIndexed { i, v ->
            val x = cx(i) - groupW / 2 + si * barW
            val topY = min(y(v), zero)
            val h = max(abs(y(v) - zero), 1.dp.toPx())
            drawRoundRect(seriesColor(si), Offset(x, topY), Size(barW, h), CornerRadius(min(3.dp.toPx(), barW / 2)))
            if (showValues) text(tm, Charts.compact(v), ValueStyle, x + barW / 2, (if (v >= 0) topY else topY + h) - 5.dp.toPx(), Align.Center)
        }
    }
}

private fun DrawScope.drawLines(spec: ChartSpec, y: (Double) -> Float, cx: (Int) -> Float) {
    val stroke = Stroke(2.5.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round)
    spec.series.forEachIndexed { si, s ->
        val path = Path()
        s.values.forEachIndexed { i, v -> if (i == 0) path.moveTo(cx(i), y(v)) else path.lineTo(cx(i), y(v)) }
        drawPath(path, seriesColor(si), style = stroke)
        if (s.values.size <= 30) s.values.forEachIndexed { i, v -> marker(markerOf(si), Offset(cx(i), y(v)), 4.5.dp.toPx(), seriesColor(si), Color.Black, 2.5.dp.toPx()) }
    }
}

private fun DrawScope.marker(kind: Marker, c: Offset, r: Float, color: Color, fill: Color, strokeW: Float) {
    val outline = Stroke(strokeW)
    when (kind) {
        Marker.Circle -> { if (fill != Color.Transparent) drawCircle(fill, r, c); drawCircle(color, r, c, style = outline) }
        Marker.Square -> { val s = Size(r * 2, r * 2); val tl = Offset(c.x - r, c.y - r); if (fill != Color.Transparent) drawRect(fill, tl, s); drawRect(color, tl, s, style = outline) }
        Marker.Diamond -> { val p = Path().apply { moveTo(c.x, c.y - r * 1.2f); lineTo(c.x + r * 1.2f, c.y); lineTo(c.x, c.y + r * 1.2f); lineTo(c.x - r * 1.2f, c.y); close() }; if (fill != Color.Transparent) drawPath(p, fill); drawPath(p, color, style = outline) }
        Marker.Triangle -> { val p = Path().apply { moveTo(c.x, c.y - r * 1.2f); lineTo(c.x + r * 1.2f, c.y + r); lineTo(c.x - r * 1.2f, c.y + r); close() }; if (fill != Color.Transparent) drawPath(p, fill); drawPath(p, color, style = outline) }
    }
}

/** Draws [s] with its baseline at [baseline] and its start, middle or end at [x]. */
private fun DrawScope.text(tm: TextMeasurer, s: String, style: TextStyle, x: Float, baseline: Float, align: Align) {
    val r: TextLayoutResult = tm.measure(s, style)
    val left = when (align) { Align.Start -> x; Align.Center -> x - r.size.width / 2f; Align.End -> x - r.size.width }
    drawText(r, topLeft = Offset(left, baseline - r.firstBaseline))
}

private fun DrawScope.drawSelection(spec: ChartSpec, tm: TextMeasurer, i: Int, x: Float, top: Float, base: Float, right: Float) {
    drawLine(Mts.Ink, Offset(x, top), Offset(x, base), 1.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(4.dp.toPx(), 3.dp.toPx())))
    val heading = spec.labels[i]
    val rows = spec.series.map { "${it.name}  ${plain(it.values[i])}" }
    val bold = ValueStyle.copy(fontSize = 13.sp)
    val widest = (listOf(tm.measure(heading, bold)) + rows.map { tm.measure(it, bold) }).maxOf { it.size.width }
    val w = widest + 20.dp.toPx()
    val h = (32 + 16 * spec.series.size).dp.toPx()
    val boxX = if (x + 10.dp.toPx() + w <= size.width) x + 10.dp.toPx() else x - 10.dp.toPx() - w
    val boxY = top + 6.dp.toPx()
    drawRoundRect(Mts.Surface200, Offset(boxX, boxY), Size(w, h), CornerRadius(8.dp.toPx()))
    drawRoundRect(Mts.Line, Offset(boxX, boxY), Size(w, h), CornerRadius(8.dp.toPx()), style = Stroke(1.dp.toPx()))
    text(tm, heading, bold.copy(color = Mts.Ink), boxX + 10.dp.toPx(), boxY + 20.dp.toPx(), Align.Start)
    rows.forEachIndexed { si, r -> text(tm, r, bold.copy(color = seriesColor(si)), boxX + 10.dp.toPx(), boxY + (38 + 16 * si).dp.toPx(), Align.Start) }
}

/** Donut or pie with its legend: label, value and share on two lines. */
@Composable
fun DonutBody(spec: ChartSpec) {
    val values = spec.series[0].values
    val total = values.sum()
    val unit = spec.yLabel
    Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
        DonutCanvas(spec, Modifier.size(120.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            spec.labels.forEachIndexed { i, label ->
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.padding(top = 3.dp).size(12.dp).background(seriesColor(i), RoundedCornerShape(3.dp)))
                    Column {
                        Text(label, style = HqType.BodySmBold.copy(fontSize = 13.sp, lineHeight = 18.sp), color = Mts.Ink)
                        Text("${Charts.compact(values[i])}${if (unit.isNotBlank()) " $unit" else ""} · ${Charts.percent(values[i], total)}%", style = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp), color = Mts.InkMuted)
                    }
                }
            }
        }
    }
}

@Composable
fun DonutCanvas(spec: ChartSpec, modifier: Modifier) {
    val measurer = rememberTextMeasurer()
    val values = spec.series[0].values
    val total = values.sum()
    val donut = spec.type == ChartType.Donut
    Canvas(modifier.semantics { contentDescription = chartDescription(spec); role = Role.Image }) {
        val c = Offset(size.width / 2, size.height / 2)
        val r = 44.dp.toPx()
        val ring = 22.dp.toPx()
        val gap = if (values.count { it > 0 } > 1) 2.dp.toPx() / r * (180f / PI.toFloat()) else 0f
        var start = -90f
        values.forEachIndexed { i, v ->
            if (v <= 0) return@forEachIndexed
            val sweep = (v / total * 360.0).toFloat()
            val drawn = max(sweep - gap, 0.5f)
            if (donut) drawArc(seriesColor(i), start + gap / 2, drawn, false, Offset(c.x - r, c.y - r), Size(r * 2, r * 2), style = Stroke(ring))
            else drawArc(seriesColor(i), start + gap / 2, drawn, true, Offset(c.x - r - ring / 2, c.y - r - ring / 2), Size((r + ring / 2) * 2, (r + ring / 2) * 2))
            start += sweep
        }
        if (donut) {
            text(measurer, Charts.compact(total), ValueStyle.copy(fontSize = 20.sp), c.x, c.y - 2.dp.toPx(), Align.Center)
            if (spec.yLabel.isNotBlank()) text(measurer, spec.yLabel, TickStyle, c.x, c.y + 14.dp.toPx(), Align.Center)
        }
    }
}

/** The board's error card: amber border, what is wrong, and the data as written so it can be checked. */
@Composable
fun ChartErrorCard(reason: String, source: String) {
    BlockFrame(border = Mts.Amber) {
        Row(
            Modifier.padding(horizontal = 12.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(HqIcons.Warning, null, Modifier.padding(top = 1.dp).size(18.dp), tint = Mts.Amber)
            Text("Could not draw this chart. ${reason.replaceFirstChar { it.uppercase() }}.", style = HqType.BodySmBold.copy(fontSize = 13.sp, lineHeight = 18.sp), color = Mts.Ink)
        }
        Spacer(Modifier.fillMaxWidth().height(1.dp).background(Mts.Hairline))
        ScrollingMono(source)
    }
}

@Composable
fun DrawingChart() {
    BlockFrame { Text("Drawing the chart…", style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(12.dp)) }
}
