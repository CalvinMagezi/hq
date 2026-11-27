package example.hq.spike.ui.notes

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/**
 * A card on the Notes home when the check found something to look at, or when HQ fixed something in
 * the last day and the fix can still be undone. Nothing at all otherwise: a card that is always
 * there becomes furniture, and one that appears means something.
 */
@Composable
internal fun VaultHealthCard(ui: NotesUi, actions: NotesActions) {
    val health = ui.health ?: return
    if (!health.worthShowing) return
    val shape = RoundedCornerShape(16.dp)
    val label = stringResource(R.string.notes_health_card_label)
    Column(
        Modifier.padding(horizontal = 16.dp, vertical = 8.dp).fillMaxWidth().clip(shape).background(Mts.Surface100).border(1.dp, Mts.Line, shape)
            .semantics { contentDescription = label }.padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        if (health.problems > 0) {
            Text(pluralStringResource(R.plurals.notes_health_title, health.problems, health.problems), style = HqType.BodyBold, color = Mts.Ink)
            if (health.dead > 0) Line(pluralStringResource(R.plurals.notes_health_dead, health.dead, health.dead))
            if (health.ambiguous > 0) Line(pluralStringResource(R.plurals.notes_health_ambiguous, health.ambiguous, health.ambiguous))
            if (health.anchors > 0) Line(pluralStringResource(R.plurals.notes_health_anchors, health.anchors, health.anchors))
            if (health.duplicates > 0) Line(pluralStringResource(R.plurals.notes_health_duplicates, health.duplicates, health.duplicates))
            if (health.frontmatter > 0) Line(pluralStringResource(R.plurals.notes_health_frontmatter, health.frontmatter, health.frontmatter))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                if (health.certain > 0) CardButton(pluralStringResource(R.plurals.notes_health_fix_count, health.certain, health.certain), !ui.healthBusy) { actions.onHealthFix(emptyList()) }
                CardButton(stringResource(R.string.notes_health_review), true, actions.onOpenHealth)
            }
        }
        if (health.problems > 0 && (health.setAside > 0 || health.unfixable >= VaultHealthParse.LEAVE_ALONE_FROM)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                if (health.setAside > 0) {
                    Text(pluralStringResource(R.plurals.notes_health_set_aside, health.setAside, health.setAside), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.weight(1f))
                    CardButton(stringResource(R.string.notes_health_count_again), !ui.healthBusy, actions.onHealthCountAgain)
                }
                if (health.unfixable >= VaultHealthParse.LEAVE_ALONE_FROM) CardButton(stringResource(R.string.notes_health_leave_alone), !ui.healthBusy, actions.onHealthSetAside)
            }
        }
        val run = health.undoRun
        if (run != null) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(pluralStringResource(R.plurals.notes_health_card_fixed, health.undoFixes, health.undoFixes), style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.weight(1f))
                CardButton(stringResource(R.string.notes_health_undo), !ui.healthBusy) { actions.onHealthUndo(run) }
            }
        }
    }
}

@Composable
private fun Line(text: String) {
    Text(text, style = HqType.BodySm, color = Mts.InkMuted)
}

@Composable
private fun CardButton(label: String, enabled: Boolean, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clip(RoundedCornerShape(24.dp)).clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
        Text(label, style = HqType.BodySmBold, color = if (enabled) Mts.BrandBlue else Mts.InkMuted)
    }
}
