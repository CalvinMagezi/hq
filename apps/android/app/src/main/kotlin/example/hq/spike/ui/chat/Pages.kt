package example.hq.spike.ui.chat

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.chat.blocks.ChartCanvas
import example.hq.spike.ui.chat.blocks.ChartSpec
import example.hq.spike.ui.chat.blocks.DonutBody
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.chat.blocks.Legend
import example.hq.spike.ui.chat.blocks.NumbersTable
import example.hq.spike.ui.chat.blocks.PAGE_CHART_HEIGHT
import example.hq.spike.ui.chat.blocks.TablePageBody
import example.hq.spike.ui.chat.blocks.numbersTable
import example.hq.spike.ui.chat.blocks.rememberCopier
import example.hq.spike.ui.chat.blocks.toTsv
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.consumeTouches
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val CaptionStyle = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp)

/** A page of its own over the whole shell: a back arrow, a title, and no bars or composer. */
@Composable
private fun PageFrame(title: String, onBack: () -> Unit, trailing: @Composable () -> Unit = {}, content: @Composable () -> Unit) {
    BackHandler(onBack = onBack)
    Column(Modifier.fillMaxSize().background(Mts.Surface000).consumeTouches().statusBarsPadding()) {
        Row(Modifier.fillMaxWidth().height(64.dp).padding(start = 4.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(0.dp)) {
            IconAction(HqIcons.ArrowBack, "Back", iconSize = 24.dp, onClick = onBack)
            Text(title, style = HqType.BodyBold, color = Mts.Ink, maxLines = 1, modifier = Modifier.weight(1f).padding(start = 4.dp))
            trailing()
        }
        Box(Modifier.weight(1f).fillMaxWidth().navigationBarsPadding()) { content() }
    }
}

private fun ChartSpec.kindLine(): String {
    val kind = "${type.title} chart"
    return when {
        type.isRound -> "$kind · ${labels.size} slices"
        else -> "$kind · ${series.size} series · touch the chart for a value"
    }
}

/** The full text of a message, which can be selected by touch. */
@Composable
fun SelectTextPage(text: String, onBack: () -> Unit) {
    PageFrame("Select text", onBack) {
        androidx.compose.foundation.text.selection.SelectionContainer {
            Text(text, style = HqType.Body, color = Mts.Ink, modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp))
        }
    }
}

/** A chart larger, with a value on touch and every number in a table under it. */
@Composable
fun ChartPage(spec: ChartSpec, onBack: () -> Unit) {
    var selected by remember(spec) { mutableStateOf<Int?>(null) }
    PageFrame(spec.title.ifBlank { "${spec.type.title} chart" }, onBack) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
            Text(spec.kindLine(), style = CaptionStyle, color = Mts.InkMuted, modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 4.dp))
            if (spec.type.isRound) {
                DonutBody(spec)
            } else {
                Legend(spec, Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
                ChartCanvas(spec, PAGE_CHART_HEIGHT, selected, { selected = it }, Modifier.padding(start = 8.dp, top = 8.dp, end = 8.dp))
            }
            Text(
                "The numbers".uppercase(), style = HqType.Group.copy(letterSpacing = 0.06.em), color = Mts.InkMuted,
                modifier = Modifier.padding(start = 16.dp, top = 12.dp, end = 16.dp, bottom = 8.dp),
            )
            NumbersTable(spec.numbersTable())
        }
    }
}

/** A table on its own page: the heading row and first column stay put, the rest scrolls both ways. Copy puts it on the clipboard for a spreadsheet. */
@Composable
fun TablePage(page: Page.Table, onBack: () -> Unit, onCopied: () -> Unit) {
    val copy = rememberCopier()
    PageFrame("Table", onBack, trailing = {
        IconAction(HqIcons.Copy, "Copy table", iconSize = 24.dp) { copy(page.table.toTsv()); onCopied() }
    }) {
        Column(Modifier.fillMaxSize()) {
            Text(page.caption, style = HqType.BodySm, color = Mts.InkMuted, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
            TablePageBody(page.table, Modifier.weight(1f))
        }
    }
}
