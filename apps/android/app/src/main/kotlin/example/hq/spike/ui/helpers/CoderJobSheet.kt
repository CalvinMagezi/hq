package example.hq.spike.ui.helpers

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.HqClient
import example.hq.spike.ui.chat.SheetFrame
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.inbox.PrimaryButton
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private const val TASK_BLOCKED = "blocked"
private const val TASK_REVIEW = "ready_for_review"
private const val RUN_PENDING = "pending"

/** One of HQ's own coding jobs: where it stands, what it has spent, and what the owner can do. */
@Composable
internal fun CoderJobSheet(run: HqClient.EdgeRun, ui: HelpersUi, actions: HelpersActions) {
    SheetFrame(actions.onJobClose, top = 90.dp) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(run.title ?: run.displayId.orEmpty(), style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                IconAction(HqIcons.Close, stringResource(R.string.close), iconSize = 24.dp, onClick = actions.onJobClose)
            }
            run.job?.let { Text(HelperWords.jobLine(example.hq.spike.ui.shell.rememberWords(), it), style = HqType.BodySm, color = Mts.InkMuted) }
            ui.githubError?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
            run.notes.forEach { Text(it, style = HqType.BodySm, color = Mts.Ink) }
            JobLinks(run, actions)
            NoteBox(ui, actions)
            JobControls(run, ui, actions)
        }
    }
}

@Composable
private fun JobLinks(run: HqClient.EdgeRun, actions: HelpersActions) {
    val context = LocalContext.current
    run.prUrl?.let { url -> PrimaryButton(stringResource(R.string.coder_open_pr), Modifier.fillMaxWidth()) { openCustomTab(context, url) } }
    OutlineButton(stringResource(R.string.coder_open_chat), Modifier.fillMaxWidth(), onClick = actions.onJobChat)
}

@Composable
private fun NoteBox(ui: HelpersUi, actions: HelpersActions) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().heightIn(min = 72.dp).clip(shape).background(Mts.Surface000).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp, vertical = 10.dp)) {
        Text(stringResource(R.string.coder_note_hint), style = HqType.Caption, color = Mts.InkMuted)
        BasicTextField(ui.jobNote, actions.onJobNote, Modifier.fillMaxWidth(), textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink))
    }
}

@Composable
private fun JobControls(run: HqClient.EdgeRun, ui: HelpersUi, actions: HelpersActions) {
    val hasNote = ui.jobNote.isNotBlank()
    when {
        run.status == RUN_PENDING -> {
            OutlineButton(stringResource(R.string.coder_send_note), Modifier.fillMaxWidth(), enabled = hasNote) { actions.onJobControl("steer") }
            OutlineButton(stringResource(R.string.coder_stop), Modifier.fillMaxWidth(), color = Mts.Amber) { actions.onJobControl("stop") }
        }
        run.taskStatus == TASK_REVIEW ->
            OutlineButton(stringResource(R.string.coder_send_back), Modifier.fillMaxWidth(), enabled = hasNote) { actions.onJobControl("feedback") }
        run.taskStatus == TASK_BLOCKED ->
            OutlineButton(stringResource(R.string.coder_retry), Modifier.fillMaxWidth()) { actions.onJobControl("retry") }
    }
}
