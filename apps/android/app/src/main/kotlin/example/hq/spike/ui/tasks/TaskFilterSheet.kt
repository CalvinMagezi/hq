package example.hq.spike.ui.tasks

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.chat.SheetFrame
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val INDENT = 16.dp

/** Narrows the All tab to a space, a folder or an initiative, with how many open tasks each holds. */
@Composable
fun TaskFilterSheet(ui: TasksUi, actions: TasksActions) {
    SheetFrame({ actions.onFilter(false) }, top = null) {
        Text("Filter", style = HqType.H2, color = Mts.Ink, modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 8.dp))
        val tree = ui.tree
        when {
            tree != null -> FilterRows(ui, tree, actions)
            ui.treeFailed -> {
                Text(TaskFilterWords.FAILED, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp))
                Box(Modifier.padding(start = 20.dp, bottom = 16.dp).heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button) { actions.onFilter(true) }.padding(horizontal = 24.dp), contentAlignment = Alignment.Center) {
                    Text("Try again", style = HqType.BodySmBold, color = Mts.OnGreen)
                }
            }
            else -> Row(Modifier.padding(horizontal = 20.dp, vertical = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                Spinner(20)
                Text(TaskFilterWords.READING, style = HqType.BodySm, color = Mts.InkMuted)
            }
        }
    }
}

@Composable
private fun FilterRows(ui: TasksUi, tree: TaskTree, actions: TasksActions) {
    val rows = TaskFilters.rows(tree, ui.tasks)
    Column(Modifier.heightIn(max = 480.dp).verticalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 4.dp)) {
        rows.forEach { row ->
            val on = row.scope == ui.scope
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(RoundedCornerShape(16.dp)).then(if (on) Modifier.background(Mts.Surface200) else Modifier)
                    .semantics { selected = on }.clickable(role = Role.RadioButton) { actions.onScope(row.scope) }
                    .padding(start = 8.dp + INDENT * (row.depth - 1).coerceAtLeast(0), end = 12.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(Modifier.size(24.dp), contentAlignment = Alignment.Center) { if (on) Icon(HqIcons.Check, null, Modifier.size(20.dp), tint = Mts.BrandGreen) }
                Text(row.label, style = if (row.depth <= 1) HqType.BodySmBold else HqType.BodySm, color = Mts.Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                Text("${row.count}", style = HqType.Caption, color = Mts.InkMuted)
            }
        }
    }
}

object TaskFilterWords {
    const val READING = "Reading your spaces"
    const val FAILED = "Could not read your spaces. HQ is not answering."

    /** The bar's line on the All tab: how many open tasks the list shows, and what it is narrowed to. */
    fun bar(open: Int, scopeName: String?): String = if (scopeName == null) "$open active" else "$open active · $scopeName"
}
