package example.hq.spike.ui.notes

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.DocTools
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** Which of the block editor's two extra tools is open. */
enum class DocTool { Table, FindReplace }

/** Asks for the table's size and hands back its column names and row count. The names are generic ("Column 1") and are edited in the note. */
@Composable
fun InsertTableDialog(onDismiss: () -> Unit, onInsert: (headers: List<String>, rows: Int) -> Unit) {
    val context = LocalContext.current
    var cols by remember { mutableStateOf("3") }
    var rows by remember { mutableStateOf("2") }
    val c = cols.toIntOrNull()?.takeIf { it in 1..DocTools.MAX_COLUMNS }
    val r = rows.toIntOrNull()?.takeIf { it in 1..DocTools.MAX_ROWS }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.docblocks_table_title), style = HqType.BodyBold, color = Mts.Ink) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(stringResource(R.string.docblocks_table_hint, DocTools.MAX_COLUMNS, DocTools.MAX_ROWS), style = HqType.BodySm, color = Mts.InkMuted)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedTextField(cols, { cols = it.filter(Char::isDigit).take(2) }, label = { Text(stringResource(R.string.docblocks_table_columns)) }, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.weight(1f))
                    OutlinedTextField(rows, { rows = it.filter(Char::isDigit).take(2) }, label = { Text(stringResource(R.string.docblocks_table_rows)) }, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.weight(1f))
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { if (c != null && r != null) onInsert((1..c).map { context.getString(R.string.docblocks_table_column, it) }, r) },
                enabled = c != null && r != null,
                modifier = Modifier.heightIn(min = 48.dp),
            ) { Text(stringResource(R.string.docblocks_table_add), style = HqType.BodyBold, color = Mts.BrandBlue) }
        },
        dismissButton = { TextButton(onDismiss, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.docblocks_cancel), style = HqType.BodyBold, color = Mts.InkMuted) } },
        containerColor = Mts.Surface100,
    )
}

/** Asks for the words to find and their replacement. [onReplace] runs the change; [message] says how it went, or why it did not. */
@Composable
fun FindReplaceDialog(busy: Boolean, message: String?, onDismiss: () -> Unit, onReplace: (find: String, with: String) -> Unit) {
    var find by remember { mutableStateOf("") }
    var with by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.docblocks_replace_title), style = HqType.BodyBold, color = Mts.Ink) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(stringResource(R.string.docblocks_replace_hint), style = HqType.BodySm, color = Mts.InkMuted)
                OutlinedTextField(find, { find = it }, label = { Text(stringResource(R.string.docblocks_replace_find)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
                OutlinedTextField(with, { with = it }, label = { Text(stringResource(R.string.docblocks_replace_with)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
                if (message != null) Text(message, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(top = 4.dp))
            }
        },
        confirmButton = {
            TextButton({ onReplace(find, with) }, enabled = !busy && find.isNotEmpty() && find != with, modifier = Modifier.heightIn(min = 48.dp)) {
                Text(stringResource(R.string.docblocks_replace_go), style = HqType.BodyBold, color = Mts.BrandBlue)
            }
        },
        dismissButton = { TextButton(onDismiss, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.docblocks_done), style = HqType.BodyBold, color = Mts.InkMuted) } },
        containerColor = Mts.Surface100,
    )
}
