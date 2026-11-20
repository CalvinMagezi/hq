package example.hq.spike.ui.notes

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import example.hq.spike.R
import example.hq.spike.data.BlockRow
import example.hq.spike.data.CommentRow
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import example.hq.spike.data.suiteAddComment
import example.hq.spike.data.suiteBlocks
import example.hq.spike.data.suiteComments
import example.hq.spike.data.suiteResolveComment
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.launch

/** What the comments page lists: the open comments first, each group in the order the note holds them. */
internal fun openFirst(comments: List<CommentRow>): List<CommentRow> = comments.sortedBy { !it.open }

/** The blocks a comment can be about: every block but another comment. */
internal fun commentable(blocks: List<BlockRow>): List<BlockRow> = blocks.filter { it.kind != "comment" }

private const val PICK_PREVIEW_LINES = 3

/** What the person is doing on the page: reading the list, choosing the block a new comment is about, or writing it. */
private sealed interface CommentStep {
    data object Reading : CommentStep
    data object Pick : CommentStep
    data class Write(val on: BlockRow) : CommentStep
}

/**
 * The note's review comments: open ones first, each with a Resolve button, and an Add button that asks which block it is about
 * and then for the words. Comments are blocks in the note, so hq writes them the way it writes any edit.
 */
@Composable
fun CommentsDialog(path: String, onClose: () -> Unit) {
    val client = HqServices.get(LocalContext.current).client
    val scope = rememberCoroutineScope()
    var comments by remember(path) { mutableStateOf<Result<List<CommentRow>>?>(null) }
    var blocks by remember(path) { mutableStateOf<List<BlockRow>>(emptyList()) }
    var step by remember(path) { mutableStateOf<CommentStep>(CommentStep.Reading) }
    var problem by remember(path) { mutableStateOf<String?>(null) }
    var tick by remember(path) { mutableIntStateOf(0) }
    val generic = stringResource(R.string.comments_failed)
    val author = stringResource(R.string.comments_author)
    LaunchedEffect(path, tick) {
        comments = runCatching { client.suiteComments(path) }
        blocks = runCatching { commentable(client.suiteBlocks(path)) }.getOrDefault(emptyList())
    }
    fun act(call: suspend () -> Unit) {
        problem = null
        scope.launch {
            runCatching { call() }
                .onSuccess { step = CommentStep.Reading; tick++ }
                .onFailure { e -> problem = (e as? HqUnavailable)?.serverMessage ?: generic }
        }
    }

    Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        DisableSelection {
            Surface(Modifier.fillMaxSize(), color = Mts.Surface000) {
                Column(Modifier.padding(top = 24.dp)) {
                    CommentsHeader(onClose, onAdd = { step = CommentStep.Pick }, canAdd = blocks.isNotEmpty())
                    if (problem != null) Text(problem.orEmpty(), style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.padding(horizontal = 16.dp))
                    CommentsList(comments, Modifier.weight(1f)) { c -> act { client.suiteResolveComment(path, c) } }
                }
            }
            when (val s = step) {
                CommentStep.Reading -> {}
                CommentStep.Pick -> BlockPicker(blocks, onDismiss = { step = CommentStep.Reading }) { step = CommentStep.Write(it) }
                is CommentStep.Write -> WriteComment(onDismiss = { step = CommentStep.Reading }) { text -> act { client.suiteAddComment(path, s.on.hash, text, author) } }
            }
        }
    }
}

@Composable
private fun CommentsHeader(onClose: () -> Unit, onAdd: () -> Unit, canAdd: Boolean) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), horizontalArrangement = Arrangement.SpaceBetween) {
        Column(Modifier.weight(1f)) {
            Text(stringResource(R.string.comments_title), style = HqType.H2, color = Mts.Ink)
            Text(stringResource(R.string.comments_hint), style = HqType.BodySm, color = Mts.InkMuted)
        }
        TextButton(onAdd, Modifier.heightIn(min = 48.dp), enabled = canAdd) { Text(stringResource(R.string.comments_add), style = HqType.BodyBold, color = Mts.BrandBlue) }
        TextButton(onClose, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.docblocks_done), style = HqType.BodyBold, color = Mts.InkMuted) }
    }
}

@Composable
private fun CommentsList(comments: Result<List<CommentRow>>?, modifier: Modifier, onResolve: (CommentRow) -> Unit) {
    val list = comments?.getOrNull()
    when {
        comments == null -> Text(stringResource(R.string.comments_loading), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
        list == null -> Text(stringResource(R.string.comments_load_failed), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(16.dp))
        list.isEmpty() -> Text(stringResource(R.string.comments_empty), style = HqType.Body, color = Mts.Ink, modifier = Modifier.padding(16.dp))
        else -> LazyColumn(modifier) { items(openFirst(list), key = { it.index to it.hash }) { CommentItem(it, onResolve) } }
    }
}

@Composable
private fun CommentItem(c: CommentRow, onResolve: (CommentRow) -> Unit) {
    val who = listOfNotNull(c.by, c.at).joinToString(" · ")
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp)) {
        if (who.isNotEmpty()) Text(who, style = HqType.Caption, color = Mts.InkMuted)
        Text(c.text, style = HqType.Body, color = if (c.open) Mts.Ink else Mts.InkMuted)
        val about = if (c.orphaned) stringResource(R.string.comments_orphaned) else c.anchor?.let { stringResource(R.string.comments_about, it) }
        if (about != null) Text(about, style = HqType.Caption, color = Mts.InkMuted, maxLines = 2, overflow = TextOverflow.Ellipsis)
        if (c.open) TextButton({ onResolve(c) }, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.comments_resolve), style = HqType.BodySmBold, color = Mts.BrandBlue) }
        else Text(stringResource(R.string.comments_resolved), style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(top = 4.dp))
    }
}

@Composable
private fun BlockPicker(blocks: List<BlockRow>, onDismiss: () -> Unit, onPick: (BlockRow) -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.comments_pick_title), style = HqType.BodyBold, color = Mts.Ink) },
        text = {
            LazyColumn {
                items(blocks, key = { it.index to it.hash }) { b ->
                    Column(Modifier.fillMaxWidth().clickable { onPick(b) }.heightIn(min = 48.dp).padding(vertical = 8.dp)) {
                        Text(b.kind, style = HqType.Caption, color = Mts.InkMuted)
                        Text(b.src, style = HqType.BodySm, color = Mts.Ink, maxLines = PICK_PREVIEW_LINES, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onDismiss, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.docblocks_cancel), style = HqType.BodyBold, color = Mts.InkMuted) } },
        containerColor = Mts.Surface100,
    )
}

@Composable
private fun WriteComment(onDismiss: () -> Unit, onSend: (String) -> Unit) {
    var text by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.comments_write_title), style = HqType.BodyBold, color = Mts.Ink) },
        text = { OutlinedTextField(text, { text = it }, minLines = 3, maxLines = 8, modifier = Modifier.fillMaxWidth()) },
        confirmButton = {
            TextButton({ onSend(text.trim()) }, Modifier.heightIn(min = 48.dp), enabled = text.isNotBlank()) { Text(stringResource(R.string.comments_send), style = HqType.BodyBold, color = Mts.BrandBlue) }
        },
        dismissButton = { TextButton(onDismiss, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.docblocks_cancel), style = HqType.BodyBold, color = Mts.InkMuted) } },
        containerColor = Mts.Surface100,
    )
}
