package example.hq.spike.ui.helpers

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import example.hq.spike.R
import example.hq.spike.data.HqClient
import example.hq.spike.ui.chat.SheetFrame
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.inbox.PrimaryButton
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private const val CUSTOM_TAB_SESSION = "android.support.customtabs.extra.SESSION"
private const val STANDING_CONNECTED = "connected"
private const val STANDING_NEEDS_INSTALL = "needs_install"
private const val STANDING_RECONNECT = "reconnect"
private const val SELECTION_ALL = "all"

/** Opens GitHub in a Custom Tab, never a WebView, so HQ cannot see the person's GitHub sign-in. */
internal fun openCustomTab(context: Context, url: String): Boolean {
    val session = Bundle().apply { putBinder(CUSTOM_TAB_SESSION, null) }
    val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url)).putExtras(session).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    return try {
        context.startActivity(intent)
        true
    } catch (e: ActivityNotFoundException) {
        false
    }
}

/** Where GitHub is connected and repositories are chosen, from the start sheet. */
@Composable
internal fun HqCoderRow(actions: HelpersActions) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape)
            .clickable(role = Role.Button, onClick = actions.onGithubOpen).padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(stringResource(R.string.github_title), style = HqType.BodyBold, color = Mts.Ink)
            Text(stringResource(R.string.github_row_body), style = HqType.BodySm, color = Mts.InkMuted)
        }
    }
}

@Composable
internal fun GithubSheet(ui: HelpersUi, actions: HelpersActions) {
    LifecycleResumeEffect(Unit) {
        actions.onGithubRefresh()
        onPauseOrDispose {}
    }
    SheetFrame(actions.onGithubClose, top = 90.dp) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.github_title), style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                IconAction(HqIcons.Close, stringResource(R.string.close), iconSize = 24.dp, onClick = actions.onGithubClose)
            }
            Text(stringResource(R.string.github_what_it_does), style = HqType.BodySm, color = Mts.InkMuted)
            ui.githubError?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
            GithubBody(ui, actions)
        }
    }
}

@Composable
private fun GithubBody(ui: HelpersUi, actions: HelpersActions) {
    val link = ui.github
    when (link?.standing) {
        null -> Text(stringResource(R.string.github_looking), style = HqType.BodySm, color = Mts.InkMuted)
        STANDING_CONNECTED -> Connected(link, ui, actions)
        STANDING_NEEDS_INSTALL -> {
            Text(stringResource(R.string.github_install_body), style = HqType.Body, color = Mts.Ink)
            Text(stringResource(R.string.github_choose_select), style = HqType.BodySm, color = Mts.Amber)
            PrimaryButton(stringResource(R.string.github_install), Modifier.fillMaxWidth(), onClick = actions.onGithubInstall)
            Disconnect(actions)
        }
        STANDING_RECONNECT -> {
            Text(stringResource(R.string.github_reconnect_body), style = HqType.Body, color = Mts.Amber)
            PrimaryButton(stringResource(R.string.github_reconnect), Modifier.fillMaxWidth(), onClick = actions.onGithubConnect)
            Disconnect(actions)
        }
        else -> {
            Text(stringResource(R.string.github_connect_body), style = HqType.Body, color = Mts.Ink)
            Text(stringResource(R.string.github_choose_select), style = HqType.BodySm, color = Mts.Amber)
            PrimaryButton(stringResource(R.string.github_connect), Modifier.fillMaxWidth(), onClick = actions.onGithubConnect)
            Text(stringResource(R.string.github_org_note), style = HqType.BodySm, color = Mts.InkMuted)
        }
    }
}

@Composable
private fun Connected(link: HqClient.GithubLink, ui: HelpersUi, actions: HelpersActions) {
    if (ui.githubJustConnected) Text(stringResource(R.string.github_just_connected), style = HqType.BodyBold, color = Mts.Ink)
    Text(stringResource(R.string.github_connected_as, link.owner.orEmpty()), style = HqType.BodyBold, color = Mts.Ink)
    if (link.selection == SELECTION_ALL) Text(stringResource(R.string.github_all_repos), style = HqType.BodySm, color = Mts.Amber)
    if (link.repos.isEmpty()) Text(stringResource(R.string.github_no_repos), style = HqType.BodySm, color = Mts.InkMuted)
    link.repos.forEach { RepoRow(it, actions) }
    if (ui.hqPaused) {
        Text(stringResource(R.string.github_paused), style = HqType.BodySm, color = Mts.Amber)
        PrimaryButton(stringResource(R.string.github_turn_on), Modifier.fillMaxWidth(), onClick = actions.onGithubTurnOn)
    }
    JobForm(link, ui, actions)
    Disconnect(actions)
}

/** Giving HQ its work here, so someone who just connected has one obvious next step. Pressing Start is the first-task yes. */
@Composable
private fun JobForm(link: HqClient.GithubLink, ui: HelpersUi, actions: HelpersActions) {
    val picked = link.repos.filter { it.picked }.map { it.repo }
    if (picked.isEmpty()) {
        if (link.repos.isNotEmpty()) Text(stringResource(R.string.github_no_picked), style = HqType.BodySm, color = Mts.InkMuted)
        return
    }
    val shape = RoundedCornerShape(12.dp)
    val chosen = HelperWords.chosenRepo(ui.githubRepo, picked)
    Text(stringResource(R.string.github_job_title), style = HqType.BodyBold, color = Mts.Ink)
    if (picked.size > 1) {
        Text(stringResource(R.string.github_pick_repo), style = HqType.BodySm, color = Mts.InkMuted)
        picked.forEach { repo ->
            Text(
                repo, style = HqType.BodySmBold, color = Mts.Ink,
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(shape).background(Mts.Surface100)
                    .border(1.dp, if (repo == chosen) Mts.BrandGreen else Mts.Line, shape)
                    .clickable(role = Role.RadioButton) { actions.onGithubRepo(repo) }.padding(horizontal = 16.dp, vertical = 14.dp),
            )
        }
    }
    Column(Modifier.fillMaxWidth().heightIn(min = 96.dp).clip(shape).background(Mts.Surface000).border(2.dp, Mts.BrandBlue, shape).padding(horizontal = 16.dp, vertical = 10.dp)) {
        Text(stringResource(R.string.github_job_hint), style = HqType.Caption, color = Mts.InkMuted)
        BasicTextField(ui.githubJob, actions.onGithubJob, Modifier.fillMaxWidth(), textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink))
    }
    Text(stringResource(R.string.github_job_example), style = HqType.BodySm, color = Mts.InkMuted)
    Text(stringResource(R.string.github_budget_each, HelperWords.dollars(ui.taskBudgetUsd)), style = HqType.BodySm, color = Mts.InkMuted)
    PrimaryButton(stringResource(R.string.github_start), Modifier.fillMaxWidth(), enabled = ui.githubJob.isNotBlank() && chosen != null, onClick = actions.onGithubStart)
    if (ui.githubStarted) Text(stringResource(R.string.github_started), style = HqType.BodySm, color = Mts.Ink)
}

@Composable
private fun RepoRow(repo: HqClient.GithubRepo, actions: HelpersActions) {
    val shape = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(repo.repo, style = HqType.BodyBold, color = Mts.Ink)
        Text(stringResource(if (repo.hasCi) R.string.github_has_ci else R.string.github_no_ci), style = HqType.BodySm, color = Mts.InkMuted)
        Text(stringResource(if (repo.protected) R.string.github_protected else R.string.github_unprotected), style = HqType.BodySm, color = Mts.InkMuted)
        if (repo.secrets.isNotEmpty()) Text(stringResource(R.string.github_uses_secrets, repo.secrets.joinToString()), style = HqType.BodySm, color = Mts.Amber)
        if (repo.previews.isNotEmpty()) Text(stringResource(R.string.github_uses_previews, repo.previews.joinToString()), style = HqType.BodySm, color = Mts.Amber)
        if (repo.flagged && !repo.picked) {
            Text(stringResource(R.string.github_flag_why), style = HqType.BodySm, color = Mts.InkMuted)
            PrimaryButton(stringResource(R.string.github_allow_repo), Modifier.fillMaxWidth()) { actions.onGithubConfirm(repo.repo) }
        }
    }
}

@Composable
private fun Disconnect(actions: HelpersActions) {
    Text(
        stringResource(R.string.github_disconnect), style = HqType.BodyBold, color = Mts.Amber,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button, onClick = actions.onGithubDisconnect).padding(vertical = 12.dp),
    )
    Text(stringResource(R.string.github_disconnect_note), style = HqType.BodySm, color = Mts.InkMuted)
}
