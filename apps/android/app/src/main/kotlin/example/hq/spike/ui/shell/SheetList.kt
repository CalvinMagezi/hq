package example.hq.spike.ui.shell

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.chat.ActionRow
import example.hq.spike.ui.chat.SheetFrame
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** One row of a [SheetList]: either something to do, or a group that opens a second list. */
sealed interface SheetItem {
    val icon: ImageVector
    val label: String
}

class SheetAction(override val icon: ImageVector, override val label: String, val hint: String? = null, val danger: Boolean = false, val onClick: () -> Unit) : SheetItem

class SheetGroup(override val icon: ImageVector, override val label: String, val items: List<SheetAction>) : SheetItem

/** The rule for every action sheet: no more than [MAX] rows, the rest folded into one group so nothing is lost. */
object SheetList {
    const val MAX = 6

    /** [items] as they will show: at most [MAX], the overflow gathered in a group named [moreLabel]. A group in the overflow keeps its own rows, flattened into it. */
    fun fit(items: List<SheetItem>, moreIcon: ImageVector, moreLabel: String): List<SheetItem> {
        if (items.size <= MAX) return items
        val keep = items.take(MAX - 1)
        val rest = items.drop(MAX - 1).flatMap { if (it is SheetGroup) it.items else listOf(it as SheetAction) }
        return keep + SheetGroup(moreIcon, moreLabel, rest)
    }
}

/** A bottom sheet of at most six rows. A group row opens its rows in place; its first row goes back. */
@Composable
fun SheetListSheet(title: String, subtitle: String?, items: List<SheetItem>, backLabel: String, onClose: () -> Unit, top: Dp = 150.dp, backIcon: ImageVector = HqIcons.ArrowBack) {
    var open by remember { mutableStateOf<SheetGroup?>(null) }
    SheetFrame(onClose, top = top) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp)) {
            Text(title, style = HqType.ScreenTitle, color = Mts.Ink, maxLines = 1)
            if (!subtitle.isNullOrBlank()) Text(subtitle, style = HqType.Caption, color = Mts.InkMuted, maxLines = 1)
        }
        Column(Modifier.verticalScroll(rememberScrollState())) {
            val group = open
            if (group != null) {
                ActionRow(backIcon, backLabel, group.label, { open = null }, minHeight = ROW_HEIGHT)
                group.items.forEach { Row(it, onClose) }
            } else {
                items.forEach { item ->
                    when (item) {
                        is SheetAction -> Row(item, onClose)
                        is SheetGroup -> ActionRow(item.icon, item.label, null, { open = item }, minHeight = ROW_HEIGHT)
                    }
                }
            }
        }
    }
}

@Composable
private fun Row(a: SheetAction, onClose: () -> Unit) {
    ActionRow(a.icon, a.label, a.hint, { onClose(); a.onClick() }, tint = if (a.danger) Mts.DangerText else Mts.Ink, minHeight = ROW_HEIGHT)
}

private val ROW_HEIGHT = 64.dp
