package example.hq.spike.ui.shell

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** Ask loses nothing but something stops (green action). Red loses something or cuts someone off (Danger action). */
enum class ConfirmKind { Ask, Red }

/**
 * One confirm dialog's words, already in the phone's language. [extra] is a third choice that loses nothing (Save changes,
 * Replace it instead) and sits above the pair.
 */
data class ConfirmWords(
    val title: String,
    val body: String,
    val keep: String,
    val act: String,
    val busy: String,
    val kind: ConfirmKind,
    val extra: String? = null,
)

/** The layout and wording rules of rules-shared section 4, as plain functions the tests can check. */
object ConfirmRules {
    private const val STACK_OVER = 18
    private const val STACK_SCALE = 1.5f
    private val EMPTY_VERBS = setOf("yes", "ok", "continue", "confirm")

    /** Side by side unless a label is long or the text is large: then safe on top, the action at the bottom. */
    fun stacks(keep: String, act: String, fontScale: Float): Boolean = maxOf(keep.length, act.length) > STACK_OVER || fontScale >= STACK_SCALE

    /** An action label names the title's verb and object, never Yes, OK, Continue or Confirm. A "Save your changes?" question ends in Discard. */
    fun actMatchesTitle(title: String, act: String): Boolean {
        val first = act.trim().substringBefore(' ').lowercase()
        if (first in EMPTY_VERBS) return false
        val verb = title.trim().substringBefore(' ').lowercase()
        return first == verb || verb == "save"
    }
}

/** The filled Danger pill with a black label: the only red on a screen, on the button that acts. */
@Composable
fun DangerButton(label: String, modifier: Modifier = Modifier, enabled: Boolean = true, busy: Boolean = false, onClick: () -> Unit) {
    Box(
        modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Danger).alpha(if (enabled) 1f else 0.5f)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) { ActLabel(label, busy, Mts.OnDanger) }
}

@Composable
private fun AskButton(label: String, modifier: Modifier, enabled: Boolean, busy: Boolean, onClick: () -> Unit) {
    Box(
        modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).alpha(if (enabled) 1f else 0.5f)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) { ActLabel(label, busy, Mts.OnGreen) }
}

@Composable
private fun ActLabel(label: String, busy: Boolean, color: Color) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        if (busy) Spinner(16, color)
        Text(label, style = HqType.BodySmBold, color = color, maxLines = 2)
    }
}

@Composable
private fun KeepButton(label: String, modifier: Modifier, enabled: Boolean, onClick: () -> Unit) {
    Box(
        modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).alpha(if (enabled) 1f else 0.5f)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, style = HqType.BodySmBold, color = Mts.Ink, maxLines = 2) }
}

/**
 * The one confirm dialog. The safe choice is on the left, the action on the right; stacked (safe on top) for a long label
 * or large text. Back, the back gesture and a tap outside choose the safe side and never act. While [busy] both buttons are
 * off; a failure keeps the dialog open with [error] in amber and the action's label unchanged.
 */
@Composable
fun ConfirmDialog(
    words: ConfirmWords,
    onKeep: () -> Unit,
    onAct: () -> Unit,
    modifier: Modifier = Modifier,
    busy: Boolean = false,
    error: String? = null,
    onExtra: (() -> Unit)? = null,
) {
    BackHandler(enabled = !busy, onBack = onKeep)
    val stacked = ConfirmRules.stacks(words.keep, words.act, LocalDensity.current.fontScale)
    Box(modifier.fillMaxSize().testTag("confirm")) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = SCRIM_ALPHA)).testTag("confirm-scrim").clickable(enabled = !busy, interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onKeep))
        Column(
            Modifier.align(Alignment.Center).padding(24.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(Mts.Surface100).border(1.dp, Mts.Hairline, RoundedCornerShape(28.dp)).padding(20.dp)
                .clickable(enabled = false) {},
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(words.title, style = HqType.H2, color = Mts.Ink)
            Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(words.body, style = HqType.BodySm, color = Mts.InkMuted)
                error?.let {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Top) {
                        Icon(HqIcons.Warning, null, Modifier.padding(top = 2.dp).size(20.dp), tint = Mts.Amber)
                        Text(it, style = HqType.BodySm, color = Mts.Amber, modifier = Modifier.testTag("confirm-error"))
                    }
                }
            }
            if (words.extra != null && onExtra != null) KeepButton(words.extra, Modifier.fillMaxWidth().testTag("confirm-extra"), !busy, onExtra)
            val act: @Composable (Modifier) -> Unit = { m ->
                val label = if (busy) words.busy else words.act
                val tagged = m.testTag("confirm-act")
                if (words.kind == ConfirmKind.Red) DangerButton(label, tagged, !busy, busy, onAct) else AskButton(label, tagged, !busy, busy, onAct)
            }
            if (stacked) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    KeepButton(words.keep, Modifier.fillMaxWidth().testTag("confirm-keep"), !busy, onKeep)
                    act(Modifier.fillMaxWidth())
                }
            } else {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    KeepButton(words.keep, Modifier.weight(1f).testTag("confirm-keep"), !busy, onKeep)
                    act(Modifier.weight(1f))
                }
            }
        }
    }
}

private const val SCRIM_ALPHA = 0.62f
