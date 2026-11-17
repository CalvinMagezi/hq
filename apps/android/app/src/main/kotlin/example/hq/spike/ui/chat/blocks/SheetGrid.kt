package example.hq.spike.ui.chat.blocks

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.HqServices
import example.hq.spike.data.SheetCell
import example.hq.spike.data.SheetView
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** The vault path of the note being read, so a `sheet` block can ask hq for its cells. Null in chat, where a sheet stays a code block. */
val LocalNotePath = compositionLocalOf<String?> { null }

/** What a fixed screen (`--es fixture n-suite-sheet`) draws instead of asking hq. */
object SheetFixture {
    var view: SheetView? = null
}

private val CellMinWidth = 72.dp
private val CellMaxWidth = 168.dp

/**
 * A `sheet` block as a grid. hq calculates and formats every cell (formulas, UGX); this only draws what it is told: the first
 * row as a header, numbers to the right, a formula or an error marked the way the cell says. At most 30 rows by 10 columns
 * arrive, so a large sheet shows its start and says how much more there is.
 */
@Composable
fun SheetBlockView(block: Block.Sheet, onCopied: () -> Unit) {
    val path = LocalNotePath.current
    val fixed = SheetFixture.view
    if (path == null && fixed == null) {
        CodeBlockView("sheet", block.source, onCopied)
        return
    }
    if (!block.closed) {
        BlockFrame { Text(stringResource(R.string.suite_sheet_drawing), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(12.dp)) }
        return
    }
    val client = HqServices.get(LocalContext.current).client
    val loaded by produceState<Result<SheetView>?>(fixed?.let { Result.success(it) }, path, block.index) {
        if (fixed == null && path != null) value = runCatching { client.suiteSheet(path, block.index) }
    }
    when (val r = loaded) {
        null -> BlockFrame { Text(stringResource(R.string.suite_sheet_drawing), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(12.dp)) }
        else -> r.fold({ SheetGrid(it) }, { CodeBlockView("sheet", block.source, onCopied) })
    }
}

@Composable
private fun SheetGrid(view: SheetView) {
    val summary = view.text
    val columns = view.rows.maxOfOrNull { it.size } ?: 0
    BlockFrame(Modifier.semantics { contentDescription = summary }) {
        // Column by column, each as wide as its widest cell, so every row lines up under the header.
        Row(Modifier.horizontalScroll(rememberScrollState())) {
            for (c in 0 until columns) {
                Column(Modifier.width(IntrinsicSize.Max)) {
                    view.rows.forEachIndexed { r, row ->
                        val header = r == 0 && view.row0 == 0
                        GridCell(row.getOrNull(c) ?: SheetCell("", false, false, false), header, Modifier.background(if (header) Mts.Surface100 else Mts.Surface000))
                    }
                }
            }
        }
        val shown = view.rows.size
        if (view.rowCount > shown + view.row0) {
            Text(
                stringResource(R.string.suite_sheet_more, shown, view.rowCount),
                style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            )
        }
    }
}

@Composable
private fun GridCell(cell: SheetCell, header: Boolean, modifier: Modifier) {
    Box(modifier.fillMaxWidth().widthIn(min = CellMinWidth, max = CellMaxWidth).heightIn(min = 40.dp).padding(horizontal = 12.dp, vertical = 8.dp), contentAlignment = if (cell.number && !header) Alignment.CenterEnd else Alignment.CenterStart) {
        Text(
            cell.text,
            style = if (header) HqType.BodySmBold else HqType.BodySm,
            color = if (cell.error) Mts.Amber else Mts.Ink,
            maxLines = 2, overflow = TextOverflow.Ellipsis,
        )
    }
}
