package example.hq.spike.ui.chat.blocks

import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
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
import kotlinx.coroutines.launch
import androidx.compose.runtime.setValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import androidx.compose.material3.TextButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.AlertDialog
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
import example.hq.spike.data.TileView
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** The vault path of the note being read, so a `sheet` block can ask hq for its cells. Null in chat, where a sheet stays a code block. */
val LocalNotePath = compositionLocalOf<String?> { null }

/** What a fixed screen (`--es fixture n-suite-sheet`) draws instead of asking hq. */
object SheetFixture {
    var view: SheetView? = null
    var tile: TileView? = null
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
    // What hq sent back after an edit replaces what was first loaded.
    var edited by remember(path, block.index) { mutableStateOf<SheetView?>(null) }
    var editing by remember { mutableStateOf<EditTarget?>(null) }
    val scope = rememberCoroutineScope()
    when (val r = loaded) {
        null -> BlockFrame { Text(stringResource(R.string.suite_sheet_drawing), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(12.dp)) }
        else -> r.fold({
            val canEdit = path != null && fixed == null
            SheetGrid(edited ?: it, canEdit) { row, col, cell -> editing = EditTarget(row, col, cell.raw) }
            editing?.let { target ->
                CellEditDialog(
                    target,
                    onDismiss = { editing = null },
                    onSave = { text, failed ->
                        scope.launch {
                            runCatching { client.suiteEditCell(path!!, block.index, target.row, target.col, target.raw, text) }
                                .onSuccess { fresh -> edited = fresh; editing = null }
                                .onFailure { failed() }
                        }
                    },
                )
            }
        }, { CodeBlockView("sheet", block.source, onCopied) })
    }
}

/** The cell being edited: its place in the sheet (zero-based) and what it held when it was opened. */
private data class EditTarget(val row: Int, val col: Int, val raw: String)

private fun columnLetters(col: Int): String {
    var n = col
    val out = StringBuilder()
    do {
        out.append('A' + n % 26)
        n = n / 26 - 1
    } while (n >= 0)
    return out.reverse().toString()
}

@Composable
private fun CellEditDialog(target: EditTarget, onDismiss: () -> Unit, onSave: (String, () -> Unit) -> Unit) {
    var text by remember(target) { mutableStateOf(target.raw) }
    var failed by remember(target) { mutableStateOf(false) }
    var saving by remember(target) { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.suite_sheet_edit_title, columnLetters(target.col) + (target.row + 1)), style = HqType.BodyBold, color = Mts.Ink) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(text, { text = it; failed = false }, singleLine = true, modifier = Modifier.fillMaxWidth())
                if (failed) Text(stringResource(R.string.suite_sheet_edit_failed), style = HqType.BodySm, color = Mts.InkMuted)
            }
        },
        confirmButton = {
            TextButton(
                onClick = { saving = true; onSave(text) { saving = false; failed = true } },
                enabled = !saving && text != target.raw,
                modifier = Modifier.heightIn(min = 48.dp),
            ) { Text(stringResource(R.string.suite_sheet_save), style = HqType.BodyBold, color = Mts.BrandBlue) }
        },
        dismissButton = { TextButton(onDismiss, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.suite_sheet_cancel), style = HqType.BodyBold, color = Mts.InkMuted) } },
        containerColor = Mts.Surface100,
    )
}

@Composable
private fun SheetGrid(view: SheetView, editable: Boolean, onEdit: (row: Int, col: Int, cell: SheetCell) -> Unit) {
    val summary = view.text
    val columns = view.rows.maxOfOrNull { it.size } ?: 0
    BlockFrame(Modifier.semantics { contentDescription = summary }) {
        // Column by column, each as wide as its widest cell, so every row lines up under the header.
        Row(Modifier.horizontalScroll(rememberScrollState())) {
            for (c in 0 until columns) {
                Column(Modifier.width(IntrinsicSize.Max)) {
                    view.rows.forEachIndexed { r, row ->
                        val header = r == 0 && view.row0 == 0
                        val cell = row.getOrNull(c) ?: SheetCell("", false, false, false)
                        GridCell(
                            cell, header,
                            Modifier.background(if (header) Mts.Surface100 else Mts.Surface000),
                            if (editable) ({ onEdit(view.row0 + r, view.col0 + c, cell) }) else null,
                        )
                    }
                }
            }
        }
        if (editable) {
            Text(stringResource(R.string.suite_sheet_edit_hint), style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp))
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
private fun GridCell(cell: SheetCell, header: Boolean, modifier: Modifier, onLongPress: (() -> Unit)?) {
    val press = if (onLongPress == null) Modifier else Modifier.combinedClickable(onLongClickLabel = stringResource(R.string.suite_sheet_edit_label), onLongClick = onLongPress, onClick = {})
    Box(modifier.then(press).fillMaxWidth().widthIn(min = CellMinWidth, max = CellMaxWidth).heightIn(min = 40.dp).padding(horizontal = 12.dp, vertical = 8.dp), contentAlignment = if (cell.number && !header) Alignment.CenterEnd else Alignment.CenterStart) {
        Text(
            cell.text,
            style = if (header) HqType.BodySmBold else HqType.BodySm,
            color = if (cell.error) Mts.Amber else Mts.Ink,
            maxLines = 2, overflow = TextOverflow.Ellipsis,
        )
    }
}

/** A `tile` block: a label over one big number, asked of hq like a sheet. Draws nothing when the number is not available. */
@Composable
fun TileBlockView(block: Block.Tile) {
    val path = LocalNotePath.current
    val fixed = SheetFixture.tile
    if (path == null && fixed == null) return
    val client = HqServices.get(LocalContext.current).client
    val loaded by produceState<TileView?>(fixed, path, block.index) {
        if (fixed == null && path != null) value = runCatching { client.suiteTile(path, block.index) }.getOrNull()
    }
    val tile = loaded ?: return
    BlockFrame {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp).semantics { contentDescription = "${tile.label} ${tile.text}" }) {
            Text(tile.label, style = HqType.BodySm, color = Mts.InkMuted)
            Text(tile.text, style = HqType.H2, color = Mts.Ink)
        }
    }
}
