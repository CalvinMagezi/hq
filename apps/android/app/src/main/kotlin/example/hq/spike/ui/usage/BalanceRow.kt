package example.hq.spike.ui.usage

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.foundation.layout.Row
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** What is left on the AI service in use, or null. The shell provides it; it no longer sits in the top bar. */
val LocalBalance = staticCompositionLocalOf<BalanceChip?> { null }

/** Opens the page that explains the balance (Spending or the AI service keys). */
val LocalOnBalance = staticCompositionLocalOf<(ChipTarget) -> Unit> { {} }

/** One line in the chats sheet: the balance, and a tap to open the page behind it. Nothing when there is no balance. */
@Composable
fun BalanceRow() {
    val chip = LocalBalance.current ?: return
    val open = LocalOnBalance.current
    val tint = when (chip.tone) { ChipTone.Normal -> Mts.InkMuted; ChipTone.Warning -> Mts.Amber; ChipTone.Exceeded -> Mts.DangerText }
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button, onClickLabel = stringResource(R.string.balance_open), onClick = { open(chip.target) })
            .padding(horizontal = 16.dp).semantics { contentDescription = chip.title },
        verticalAlignment = Alignment.CenterVertically,
    ) { Text(chip.label, style = HqType.Caption, color = tint, maxLines = 1, overflow = TextOverflow.Ellipsis) }
}
