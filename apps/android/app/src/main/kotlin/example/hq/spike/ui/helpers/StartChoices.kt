package example.hq.spike.ui.helpers

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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.ui.inbox.PrimaryButton
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.rememberWords
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private const val CHIP_NAME_LINES = 1

/** Every helper that can really start from here: HQ Coder, then what the chosen computer has installed. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun WhichHelper(form: StartForm, actions: HelpersActions) {
    val w = rememberWords()
    val shape = RoundedCornerShape(12.dp)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        HelperWords.startChoices(form).forEach { key ->
            val label = if (key == HelperWords.HQ_CODER) stringResource(R.string.helpers_coder_name) else HelperWords.agentName(w, key)
            val on = form.harness == key
            Box(
                Modifier.heightIn(min = 48.dp).clip(shape).background(Mts.Surface100).border(1.dp, if (on) Mts.BrandGreen else Mts.Line, shape)
                    .clickable(role = Role.RadioButton, onClickLabel = label) { actions.onHarness(key) }.padding(horizontal = 16.dp, vertical = 12.dp),
                contentAlignment = Alignment.CenterStart,
            ) { Text(label, style = HqType.BodyBold, color = Mts.Ink, maxLines = CHIP_NAME_LINES) }
        }
    }
    if (form.host != null && form.harness != HelperWords.HQ_CODER && form.chosenAgents().isEmpty()) {
        Text(stringResource(R.string.helpers_agents_none), style = HqType.BodySm, color = Mts.Amber)
    }
}

/** What HQ Coder needs instead of a computer: GitHub connected and one of the person's repositories. */
@Composable
internal fun CoderChoices(form: StartForm, ui: HelpersUi, actions: HelpersActions) {
    val shape = RoundedCornerShape(12.dp)
    val picked = ui.github?.repos.orEmpty().filter { it.picked }.map { it.repo }
    HqCoderRow(actions)
    Text(stringResource(R.string.helpers_coder_no_computer), style = HqType.BodySm, color = Mts.InkMuted)
    if (!HelperWords.coderReady(ui.github)) {
        Text(stringResource(R.string.helpers_coder_connect), style = HqType.BodySm, color = Mts.Amber)
        return
    }
    if (picked.size > 1) {
        Text(stringResource(R.string.helpers_coder_which_repo), style = HqType.BodySmBold, color = Mts.Ink)
        val chosen = HelperWords.chosenRepo(form.repo?.id, picked)
        picked.forEach { repo ->
            Text(
                repo, style = HqType.BodySmBold, color = Mts.Ink,
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(shape).background(Mts.Surface100)
                    .border(1.dp, if (repo == chosen) Mts.BrandGreen else Mts.Line, shape)
                    .clickable(role = Role.RadioButton) { actions.onAttachPick(AttachKind.Repo, Attached(repo, repo)) }.padding(horizontal = 16.dp, vertical = 14.dp),
            )
        }
    }
    if (ui.hqPaused) {
        Text(stringResource(R.string.github_paused), style = HqType.BodySm, color = Mts.Amber)
        PrimaryButton(stringResource(R.string.github_turn_on), Modifier.fillMaxWidth(), onClick = actions.onGithubTurnOn)
    }
    Text(stringResource(R.string.github_budget_each, HelperWords.dollars(ui.taskBudgetUsd)), style = HqType.BodySm, color = Mts.InkMuted)
    form.error?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
    PrimaryButton(
        stringResource(if (form.busy) R.string.helpers_starting else R.string.helpers_start), Modifier.fillMaxWidth(),
        enabled = HelperWords.canStartCoder(form, ui.github) && !form.busy, onClick = actions.onStartSubmit,
    )
}

/** A note, a task or a repository the helper starts connected to. HQ Coder picks its repository above, so it has no repository chip. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun AttachChoices(form: StartForm, actions: HelpersActions) {
    val kinds = AttachKind.entries.filter { it != AttachKind.Repo || form.harness != HelperWords.HQ_CODER }
    Text(stringResource(R.string.helpers_attach_title), style = HqType.BodySmBold, color = Mts.InkMuted)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        kinds.forEach { AttachChip(it, form, actions) }
    }
    form.picking?.let { AttachPicker(it, form, actions) }
}

@Composable
private fun AttachChip(kind: AttachKind, form: StartForm, actions: HelpersActions) {
    val shape = RoundedCornerShape(12.dp)
    val attached = form.attached(kind)
    val label = attached?.title ?: stringResource(kindWords(kind))
    val border = if (attached != null || form.picking == kind) Mts.BrandGreen else Mts.Line
    Row(
        Modifier.heightIn(min = 48.dp).clip(shape).background(Mts.Surface100).border(1.dp, border, shape)
            .clickable(role = Role.Button, onClickLabel = label) { if (attached != null) actions.onAttachClear(kind) else actions.onAttachOpen(kind) }.padding(horizontal = 14.dp, vertical = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, style = HqType.BodySmBold, color = Mts.Ink, maxLines = CHIP_NAME_LINES, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
        if (attached != null) Icon(HqIcons.Close, stringResource(R.string.helpers_attach_remove, attached.title), Modifier.size(16.dp), tint = Mts.InkMuted)
    }
}

@Composable
private fun AttachPicker(kind: AttachKind, form: StartForm, actions: HelpersActions) {
    val shape = RoundedCornerShape(12.dp)
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(shape).background(Mts.Surface000).border(1.dp, Mts.Line, shape).padding(horizontal = 16.dp, vertical = 12.dp)) {
            if (form.pickQuery.isEmpty()) Text(stringResource(hintWords(kind)), style = HqType.Body, color = Mts.InkMuted)
            BasicTextField(form.pickQuery, actions.onAttachQuery, Modifier.fillMaxWidth(), textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink), singleLine = true)
        }
        if (form.pickQuery.isNotBlank() && form.pickResults.isEmpty()) Text(stringResource(R.string.helpers_attach_none), style = HqType.BodySm, color = Mts.InkMuted)
        form.pickResults.forEach { found ->
            Text(
                found.title, style = HqType.BodySmBold, color = Mts.Ink, maxLines = CHIP_NAME_LINES, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(shape).background(Mts.Surface100)
                    .clickable(role = Role.Button) { actions.onAttachPick(kind, found) }.padding(horizontal = 16.dp, vertical = 14.dp),
            )
        }
    }
}

private fun kindWords(kind: AttachKind) = when (kind) {
    AttachKind.Note -> R.string.helpers_attach_note
    AttachKind.Task -> R.string.helpers_attach_task
    AttachKind.Repo -> R.string.helpers_attach_repo
}

private fun hintWords(kind: AttachKind) = when (kind) {
    AttachKind.Note -> R.string.helpers_attach_search_notes
    AttachKind.Task -> R.string.helpers_attach_search_tasks
    AttachKind.Repo -> R.string.helpers_attach_repo_hint
}
