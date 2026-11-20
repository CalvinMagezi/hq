package example.hq.spike.ui.helpers

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.LifecycleResumeEffect
import example.hq.spike.R
import example.hq.spike.data.SandboxGrant
import example.hq.spike.ui.chat.SheetFrame
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.inbox.SandboxGrantWords
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val SmallText = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)

/** Filters a session's grants down to decided per-session resource grants (active and past history). */
internal fun visibleSessionGrants(grants: List<SandboxGrant>): List<SandboxGrant> =
    grants.filter { it.isResource && !it.isStandingRule && !it.isPending }

@Composable
internal fun AccessRow(onOpen: () -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(shape).background(Mts.Surface100)
            .clickable(role = Role.Button, onClick = onOpen).padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(HqIcons.Shield, null, Modifier.size(22.dp), tint = Mts.Ink)
        Text(stringResource(R.string.grants_timeline_title), style = HqType.BodyBold, color = Mts.Ink, modifier = Modifier.weight(1f))
        Icon(HqIcons.ChevronRight, null, Modifier.size(20.dp), tint = Mts.InkMuted)
    }
}

@Composable
internal fun GrantsSheet(open: HelperOpen, actions: HelpersActions) {
    LifecycleResumeEffect(open.id) {
        actions.onGrantsRefresh()
        onPauseOrDispose {}
    }
    val rows = visibleSessionGrants(open.grants)
    val resources = LocalContext.current.resources
    SheetFrame(actions.onGrantsClose, top = 90.dp) {
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.grants_timeline_title), style = HqType.H2, color = Mts.Ink, modifier = Modifier.weight(1f))
                IconAction(HqIcons.Close, stringResource(R.string.close), iconSize = 24.dp, onClick = actions.onGrantsClose)
            }
            open.grantsError?.let { Text(it, style = HqType.BodySm, color = Mts.Amber) }
            open.grantsSummary?.let { summary ->
                Text(
                    SandboxGrantWords.formatSummary(summary) { res, args -> resources.getString(res, *args) },
                    style = HqType.BodySm,
                    color = Mts.InkMuted,
                )
            }
            if (open.standingRules.isNotEmpty() || open.grantsSummary != null) {
                Text(stringResource(R.string.grants_settings_rules), style = HqType.Group, color = Mts.InkMuted)
                if (open.standingRules.isEmpty()) {
                    Text(stringResource(R.string.grants_settings_rules_empty), style = HqType.BodySm, color = Mts.InkMuted)
                } else {
                    open.standingRules.forEach { rule ->
                        StandingRuleRow(rule, open.revokingGrantId, actions.onGrantRevoke)
                    }
                }
                Text(stringResource(R.string.grants_settings_active), style = HqType.Group, color = Mts.InkMuted)
            }
            when {
                rows.isEmpty() && open.grantsLoading -> Text(stringResource(R.string.grants_loading), style = HqType.BodySm, color = Mts.InkMuted)
                rows.isEmpty() -> Text(stringResource(R.string.grants_session_empty), style = HqType.BodySm, color = Mts.InkMuted)
                else -> rows.forEach { grant ->
                    if (grant.isActive) ActiveGrantRow(grant, open.revokingGrantId, actions.onGrantRevoke)
                    else MutedGrantRow(grant)
                }
            }
        }
    }
}

@Composable
private fun StandingRuleRow(rule: SandboxGrant, revokingId: String?, onRevoke: (String) -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    val busy = revokingId == rule.id
    val (res, target, resource) = SandboxGrantWords.standingRuleFormat(rule)
    Column(
        Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(stringResource(res, target, resource), style = HqType.BodySmBold, color = Mts.Ink)
        if (rule.decisionNote.isNotBlank()) {
            Text(rule.decisionNote, style = SmallText, color = Mts.InkMuted)
        }
        OutlineButton(
            label = stringResource(if (busy) R.string.grants_revoking else R.string.grants_revoke),
            modifier = Modifier.fillMaxWidth().alpha(if (revokingId == null) 1f else 0.5f),
            enabled = revokingId == null,
            color = Mts.Danger,
            onClick = { onRevoke(rule.id) },
        )
    }
}

@Composable
private fun ActiveGrantRow(grant: SandboxGrant, revokingId: String?, onRevoke: (String) -> Unit) {
    val shape = RoundedCornerShape(12.dp)
    val busy = revokingId == grant.id
    val label = stringResource(
        if (grant.decidedBy == "policy") R.string.grants_hq_allowed else R.string.grants_owner_allowed,
        grant.resource,
    )
    Column(
        Modifier.fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape).padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(label, style = HqType.BodySmBold.copy(fontFamily = FontFamily.Monospace), color = Mts.Ink)
        Text(stringResource(SandboxGrantWords.modeRes(grant.capability)), style = SmallText, color = Mts.InkMuted)
        OutlineButton(
            label = stringResource(if (busy) R.string.grants_revoking else R.string.grants_revoke),
            modifier = Modifier.fillMaxWidth().alpha(if (revokingId == null) 1f else 0.5f),
            enabled = revokingId == null,
            color = Mts.Danger,
            onClick = { onRevoke(grant.id) },
        )
    }
}

@Composable
private fun MutedGrantRow(grant: SandboxGrant) {
    val shape = RoundedCornerShape(12.dp)
    val stateLabel = stringResource(SandboxGrantWords.stateRes(grant.state))
    Column(
        Modifier.fillMaxWidth().clip(shape).background(Mts.Surface000).border(1.dp, Mts.Hairline, shape).padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text("$stateLabel: ${grant.resource}", style = HqType.BodySm.copy(fontFamily = FontFamily.Monospace), color = Mts.InkMuted)
        if (grant.decisionNote.isNotBlank()) {
            Text(grant.decisionNote, style = SmallText, color = Mts.InkMuted)
        }
    }
}
