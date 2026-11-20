package example.hq.spike.ui.notes

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import example.hq.spike.R
import example.hq.spike.data.CardBody
import example.hq.spike.data.CardChange
import example.hq.spike.data.CardRow
import example.hq.spike.data.CardStyles
import example.hq.spike.data.HqServices
import example.hq.spike.data.HqUnavailable
import example.hq.spike.data.SlideBodies
import example.hq.spike.data.suiteAddCard
import example.hq.spike.data.suiteDeleteCard
import example.hq.spike.data.suiteMoveCard
import example.hq.spike.data.suiteSaveCard
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts
import kotlinx.coroutines.launch

/** The words for each layout hq offers. */
private fun layoutLabel(layout: String): Int = if (layout == "two-column") R.string.slides_layout_two_column else R.string.slides_layout_content

/** The words for each transition hq offers; null is none chosen. */
private fun transitionLabel(kind: String?): Int = when (kind) {
    "none" -> R.string.slides_transition_none
    "fade" -> R.string.slides_transition_fade
    "push" -> R.string.slides_transition_push
    "wipe" -> R.string.slides_transition_wipe
    "split" -> R.string.slides_transition_split
    "cover" -> R.string.slides_transition_cover
    "dissolve" -> R.string.slides_transition_dissolve
    else -> R.string.slides_transition_default
}

/** What the editor's fields hold, kept together so "has anything changed" is one comparison. */
private data class CardFields(val title: String, val text: String, val notes: String, val layout: String, val transition: String?)

private fun fieldsOf(card: CardRow?, body: CardBody?) = CardFields(
    card?.title.orEmpty(), body?.text.orEmpty(), card?.notes?.joinToString("\n").orEmpty(), card?.layout ?: CardStyles.layouts.first(), CardStyles.transitionKind(card?.transition),
)

/**
 * The card editor: title, body (markdown), speaker notes, layout and transition, with move up, move down and delete for a card that
 * exists. [card] is null for a new card at the end. Every change goes to hq with the card's hash, so a card that changed since is refused.
 * [onClose] says whether anything changed, so the deck is asked for again.
 */
@Composable
fun CardEditor(path: String, card: CardRow?, deck: DeckData, onClose: (changed: Boolean) -> Unit, onNotice: (String) -> Unit) {
    val client = HqServices.get(LocalContext.current).client
    val scope = rememberCoroutineScope()
    val body = remember(card?.hash, deck.blocks) { card?.let { SlideBodies.bodyOf(deck.blocks, it.index) } }
    val start = remember(card?.hash, deck.blocks) { fieldsOf(card, body) }
    var fields by remember(start) { mutableStateOf(start) }
    var busy by remember(start) { mutableStateOf(false) }
    var problem by remember(start) { mutableStateOf<String?>(null) }
    var confirmDelete by remember(start) { mutableStateOf(false) }
    val generic = stringResource(R.string.slides_failed)
    val saved = stringResource(R.string.slides_saved)

    fun act(call: suspend () -> Unit) {
        busy = true
        problem = null
        scope.launch {
            runCatching { call() }
                .onSuccess { onNotice(saved); onClose(true) }
                .onFailure { e -> problem = (e as? HqUnavailable)?.serverMessage ?: generic; busy = false }
        }
    }
    fun save() = act {
        val markdown = (body ?: CardBody("", emptyList())).markdown(fields.text)
        if (card == null) client.suiteAddCard(path, fields.title.trim(), markdown, null, null)
        else client.suiteSaveCard(path, card, CardChange(fields.title.trim(), markdown, fields.notes.trim(), fields.layout, fields.transition), fields.text != start.text)
    }

    Dialog(onDismissRequest = { if (!busy) onClose(false) }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        DisableSelection {
            Surface(Modifier.fillMaxSize(), color = Mts.Surface000) {
                Column(Modifier.padding(top = 24.dp)) {
                    EditorHeader(card, fields != start, busy, fields.title.isNotBlank(), onCancel = { onClose(false) }, onSave = ::save)
                    EditorFields(fields, { fields = it }, problem, Modifier.weight(1f))
                    if (card != null) MoveRow(card, deck.cards.size, fields != start, busy, { to -> act { client.suiteMoveCard(path, card, to) } }, { confirmDelete = true })
                }
            }
            if (confirmDelete && card != null) DeleteConfirm(card.title, onNo = { confirmDelete = false }) { confirmDelete = false; act { client.suiteDeleteCard(path, card) } }
        }
    }
}

@Composable
private fun EditorHeader(card: CardRow?, dirty: Boolean, busy: Boolean, valid: Boolean, onCancel: () -> Unit, onSave: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(
            if (card == null) stringResource(R.string.slides_new_title) else stringResource(R.string.slides_edit_title, card.index + 1),
            style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f).padding(top = 8.dp),
        )
        TextButton(onCancel, Modifier.heightIn(min = 48.dp), enabled = !busy) { Text(stringResource(R.string.slides_close), style = HqType.BodyBold, color = Mts.InkMuted) }
        TextButton(onSave, Modifier.heightIn(min = 48.dp), enabled = !busy && valid && (dirty || card == null)) { Text(stringResource(R.string.slides_save), style = HqType.BodyBold, color = Mts.BrandBlue) }
    }
}

@Composable
private fun EditorFields(fields: CardFields, set: (CardFields) -> Unit, problem: String?, modifier: Modifier) {
    Column(modifier.verticalScroll(rememberScrollState()).padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedTextField(fields.title, { set(fields.copy(title = it.replace("\n", " "))) }, label = { Text(stringResource(R.string.slides_field_title)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(fields.text, { set(fields.copy(text = it)) }, label = { Text(stringResource(R.string.slides_field_body)) }, minLines = 5, maxLines = 14, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(fields.notes, { set(fields.copy(notes = it)) }, label = { Text(stringResource(R.string.slides_field_notes)) }, minLines = 2, maxLines = 8, modifier = Modifier.fillMaxWidth())
        Text(stringResource(R.string.slides_field_layout), style = HqType.BodySmBold, color = Mts.Ink)
        Choices(CardStyles.layouts, fields.layout, { stringResource(layoutLabel(it)) }) { set(fields.copy(layout = it)) }
        Text(stringResource(R.string.slides_field_transition), style = HqType.BodySmBold, color = Mts.Ink)
        Choices(CardStyles.transitions, fields.transition, { stringResource(transitionLabel(it)) }) { set(fields.copy(transition = it)) }
        if (problem != null) Text(problem, style = HqType.BodySm, color = Mts.Amber)
    }
}

@Composable
private fun MoveRow(card: CardRow, count: Int, dirty: Boolean, busy: Boolean, onMove: (Int) -> Unit, onDelete: () -> Unit) {
    Column(Modifier.padding(horizontal = 8.dp)) {
        if (dirty) Text(stringResource(R.string.slides_save_first), style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 8.dp))
        Row {
            val free = !busy && !dirty
            TextButton({ onMove(card.index - 1) }, Modifier.heightIn(min = 48.dp), enabled = free && card.index > 0) { Text(stringResource(R.string.slides_move_up), style = HqType.BodySmBold, color = Mts.BrandBlue) }
            TextButton({ onMove(card.index + 1) }, Modifier.heightIn(min = 48.dp), enabled = free && card.index < count - 1) { Text(stringResource(R.string.slides_move_down), style = HqType.BodySmBold, color = Mts.BrandBlue) }
            TextButton(onDelete, Modifier.heightIn(min = 48.dp), enabled = !busy) { Text(stringResource(R.string.slides_delete), style = HqType.BodySmBold, color = Mts.InkMuted) }
        }
    }
}

@Composable
private fun DeleteConfirm(title: String, onNo: () -> Unit, onYes: () -> Unit) {
    AlertDialog(
        onDismissRequest = onNo,
        title = { Text(stringResource(R.string.slides_delete_title, title), style = HqType.BodyBold, color = Mts.Ink) },
        text = { Text(stringResource(R.string.slides_delete_body), style = HqType.BodySm, color = Mts.InkMuted) },
        confirmButton = { TextButton(onYes, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.slides_delete), style = HqType.BodyBold, color = Mts.BrandBlue) } },
        dismissButton = { TextButton(onNo, Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.docblocks_cancel), style = HqType.BodyBold, color = Mts.InkMuted) } },
        containerColor = Mts.Surface100,
    )
}
