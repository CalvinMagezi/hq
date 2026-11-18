package example.hq.spike.ui.notes

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import example.hq.spike.R
import example.hq.spike.data.BlockRow
import example.hq.spike.data.HqServices
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.launch

/** What the person is doing to the list: changing one block, or adding one after a block (or at the end, or at the top). */
private sealed interface BlockEdit {
    data class Change(val row: BlockRow) : BlockEdit
    data class Add(val after: BlockRow?) : BlockEdit
}

private const val PREVIEW_LINES = 8

/**
 * Edits a note one block at a time: each heading, paragraph, list, table or code block is a row that opens as markdown. Every
 * change goes to hq with the hash of the block the phone saw, so a block that changed since is refused and nothing else in the
 * note moves; the note itself is the history. [onClose] says whether anything changed, so the reader can reload.
 */
@Composable
fun DocBlocksEditor(path: String, onClose: (changed: Boolean) -> Unit) {
    val client = HqServices.get(LocalContext.current).client
    val scope = rememberCoroutineScope()
    var blocks by remember(path) { mutableStateOf<Result<List<BlockRow>>?>(null) }
    var changed by remember(path) { mutableStateOf(false) }
    var editing by remember { mutableStateOf<BlockEdit?>(null) }
    LaunchedEffect(path) { blocks = runCatching { client.suiteBlocks(path) } }

    Dialog(onDismissRequest = { onClose(changed) }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        DisableSelection {
            Surface(Modifier.fillMaxSize(), color = Mts.Surface000) {
                Column(Modifier.padding(top = 24.dp)) {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), horizontalArrangement = Arrangement.SpaceBetween) {
                        Column(Modifier.weight(1f)) {
                            Text(stringResource(R.string.docblocks_title), style = HqType.H2, color = Mts.Ink)
                            Text(stringResource(R.string.docblocks_hint), style = HqType.BodySm, color = Mts.InkMuted)
                        }
                        TextButton({ onClose(changed) }, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.docblocks_done), style = HqType.BodyBold, color = Mts.BrandBlue) }
                    }
                    val list = blocks?.getOrNull()
                    when {
                        blocks == null -> Text(stringResource(R.string.docblocks_loading), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
                        list == null -> Text(stringResource(R.string.docblocks_load_failed), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
                        else -> LazyColumn(Modifier.weight(1f)) {
                            if (list.isEmpty()) item { Text(stringResource(R.string.docblocks_empty), style = HqType.Body, color = Mts.Ink, modifier = Modifier.padding(16.dp)) }
                            itemsIndexed(list, key = { _, b -> b.index to b.hash }) { _, row ->
                                Column(Modifier.fillMaxWidth().clickable { editing = BlockEdit.Change(row) }.padding(horizontal = 16.dp, vertical = 10.dp)) {
                                    Text(row.kind, style = HqType.Caption, color = Mts.InkMuted)
                                    Text(row.src, style = HqType.BodySm, color = Mts.Ink, maxLines = PREVIEW_LINES, overflow = TextOverflow.Ellipsis)
                                    TextButton({ editing = BlockEdit.Add(row) }, Modifier.heightIn(min = 40.dp)) { Text(stringResource(R.string.docblocks_add_below), style = HqType.BodySm, color = Mts.BrandBlue) }
                                }
                            }
                            item {
                                TextButton({ editing = BlockEdit.Add(list.lastOrNull()) }, Modifier.padding(horizontal = 8.dp).heightIn(min = 48.dp)) {
                                    Text(stringResource(R.string.docblocks_add_end), style = HqType.BodyBold, color = Mts.BrandBlue)
                                }
                            }
                        }
                    }
                }
            }

            editing?.let { target ->
                val start = (target as? BlockEdit.Change)?.row?.src.orEmpty()
                var field by remember(target) { mutableStateOf(TextFieldValue(start, TextRange(start.length))) }
                val text = field.text
                var failed by remember(target) { mutableStateOf(false) }
                var busy by remember(target) { mutableStateOf(false) }
                fun run(call: suspend () -> List<BlockRow>) {
                    busy = true
                    scope.launch {
                        runCatching { call() }
                            .onSuccess { fresh -> blocks = Result.success(fresh); changed = true; editing = null }
                            .onFailure { failed = true; busy = false }
                    }
                }
                AlertDialog(
                    onDismissRequest = { editing = null },
                    title = {
                        val title = when (target) {
                            is BlockEdit.Change -> stringResource(R.string.docblocks_edit_title, target.row.kind)
                            is BlockEdit.Add -> stringResource(R.string.docblocks_add_title)
                        }
                        Text(title, style = HqType.BodyBold, color = Mts.Ink)
                    },
                    text = {
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            OutlinedTextField(field, { field = it; failed = false }, minLines = 3, maxLines = 12, modifier = Modifier.fillMaxWidth())
                            // Below the field: the selection's Cut and Copy bar floats above the text and would cover it.
                            FormatToolbar { field = it(field); failed = false }
                            if (failed) Text(stringResource(R.string.docblocks_failed), style = HqType.BodySm, color = Mts.InkMuted)
                        }
                    },
                    confirmButton = {
                        TextButton(
                            onClick = {
                                when (target) {
                                    is BlockEdit.Change -> run { client.suiteEditBlock(path, target.row.index, target.row.hash, text) }
                                    is BlockEdit.Add -> run { client.suiteInsertBlock(path, target.after?.index, target.after?.hash, text) }
                                }
                            },
                            enabled = !busy && text.isNotBlank() && text != start,
                            modifier = Modifier.heightIn(min = 48.dp),
                        ) { Text(stringResource(R.string.docblocks_save), style = HqType.BodyBold, color = Mts.BrandBlue) }
                    },
                    dismissButton = {
                        Row {
                            if (target is BlockEdit.Change) {
                                TextButton({ run { client.suiteDeleteBlock(path, target.row.index, target.row.hash) } }, Modifier.heightIn(min = 48.dp), enabled = !busy) {
                                    Text(stringResource(R.string.docblocks_delete), style = HqType.BodyBold, color = Mts.InkMuted)
                                }
                            }
                            TextButton({ editing = null }, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.docblocks_cancel), style = HqType.BodyBold, color = Mts.InkMuted) }
                        }
                    },
                    containerColor = Mts.Surface100,
                )
            }
        }
    }
}

/** The formatting buttons above the block's text: each one rewrites the field's text and selection. */
@Composable
private fun FormatToolbar(apply: ((TextFieldValue) -> TextFieldValue) -> Unit) {
    val buttons = listOf<Pair<Int, (TextFieldValue) -> TextFieldValue>>(
        R.string.docblocks_fmt_bold to { it.bold() },
        R.string.docblocks_fmt_italic to { it.italic() },
        R.string.docblocks_fmt_code to { it.code() },
        R.string.docblocks_fmt_link to { it.link() },
        R.string.docblocks_fmt_heading to { it.toggleLines(LineStyle.Heading(2)) },
        R.string.docblocks_fmt_bullets to { it.toggleLines(LineStyle.Bullet) },
        R.string.docblocks_fmt_numbers to { it.toggleLines(LineStyle.Numbered) },
        R.string.docblocks_fmt_quote to { it.toggleLines(LineStyle.Quote) },
    )
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) {
        buttons.forEach { (label, change) ->
            TextButton({ apply(change) }, Modifier.heightIn(min = 48.dp)) {
                Text(stringResource(label), style = HqType.BodySmBold, color = Mts.BrandBlue)
            }
        }
    }
}
