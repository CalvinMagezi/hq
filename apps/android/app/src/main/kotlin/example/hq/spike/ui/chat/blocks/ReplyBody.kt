package example.hq.spike.ui.chat.blocks

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val BlockGap = 12.dp
private val HeadingGap = 8.dp
private val QuoteGap = 4.dp
private const val BIG_HEADING_MAX_LEVEL = 2

/** What a tap on a rich block can ask for. Plain functions so a block can be drawn from a fixed state. */
class BlockActions(
    val onOpenNote: (String) -> Unit = {},
    val onOpenChart: (ChartSpec) -> Unit = {},
    val onOpenTable: (Block.Table) -> Unit = {},
    val onOpenTasks: () -> Unit = {},
    val onCopied: () -> Unit = {},
    val onOpenFile: (Attachment) -> Unit = {},
    val chipState: (String) -> ChipState = { ChipState.Ready },
)

@Composable
fun BlockView(block: Block, actions: BlockActions) {
    when (block) {
        is Block.Paragraph -> RichText(block.text, HqType.Body, Mts.Ink, actions.onOpenNote)
        is Block.Heading -> RichText(block.text, if (block.level <= BIG_HEADING_MAX_LEVEL) HqType.H2 else HqType.BodyBold, Mts.Ink, actions.onOpenNote)
        is Block.Quote -> QuoteView { RichText(block.text, HqType.Body, Mts.InkMuted, actions.onOpenNote) }
        is Block.Bullets -> ListView(block.ordered, block.items, actions.onOpenNote)
        is Block.Checklist -> ChecklistView(block.items, actions.onOpenNote)
        is Block.Code -> CodeBlockView(block.lang, block.text, actions.onCopied)
        is Block.ChartBlock -> ChartBlockView(block, actions)
        is Block.Table -> TableBlockView(block) { actions.onOpenTable(block) }
        is Block.Image -> BlockedImageView()
    }
}

@Composable
private fun ChartBlockView(block: Block.ChartBlock, actions: BlockActions) {
    val result = block.result
    when {
        !block.closed -> DrawingChart()
        result is ChartParse.Ok -> ChartCard(result.spec) { actions.onOpenChart(result.spec) }
        result is ChartParse.Bad -> ChartErrorCard(result.reason, block.source)
    }
}

/** Blocks one under another: 8 dp under a heading, 4 dp under a quote, 12 dp between the rest. */
@Composable
fun Blocks(blocks: List<Block>, actions: BlockActions, modifier: Modifier = Modifier) {
    Column(modifier) {
        blocks.forEachIndexed { i, block ->
            val gap = when { i == 0 -> 0.dp; blocks[i - 1] is Block.Heading -> HeadingGap; blocks[i - 1] is Block.Quote -> QuoteGap; else -> BlockGap }
            Box(Modifier.padding(top = gap)) { BlockView(block, actions) }
        }
    }
}

/** A reply's text as blocks, in the order it was written. */
@Composable
fun ReplyBody(text: String, actions: BlockActions, modifier: Modifier = Modifier) {
    val blocks = remember(text) { parseBlocks(text) }
    Blocks(blocks, actions, modifier)
}

/** The notes and tasks a finished reply made, under its text. */
@Composable
fun ReplyCards(cards: StepCards, age: String, actions: BlockActions, modifier: Modifier = Modifier) {
    if (cards.isEmpty) return
    Column(modifier, verticalArrangement = Arrangement.spacedBy(BlockGap)) {
        cards.written.forEach { NoteCardView(it, age) { actions.onOpenNote(it.path) } }
        if (cards.read.isNotEmpty()) ReadNotes(cards.read, actions.chipState, actions.onOpenNote)
        cards.tasks.forEach { TaskCardView(it, actions.onOpenTasks) }
    }
}
