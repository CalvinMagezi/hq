package example.hq.spike.ui.chat.blocks

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val MonoStyle = HqType.Caption.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp, lineHeight = 18.sp)
private const val FADE_DP = 36

/** Mono text that does not wrap: it scrolls sideways, with a fade at the right edge that says there is more. */
@Composable
fun ScrollingMono(text: String, modifier: Modifier = Modifier) {
    val scroll = rememberScrollState()
    Box(modifier.fillMaxWidth()) {
        Text(text, style = MonoStyle, color = Mts.Ink, softWrap = false, modifier = Modifier.horizontalScroll(scroll).padding(horizontal = 12.dp, vertical = 10.dp))
        if (scroll.value < scroll.maxValue) RightFade()
    }
}

/**
 * 36 dp wide, transparent to black, ignores touch; sits over the right edge of anything that scrolls sideways.
 * It takes its height from the parent box (matchParentSize), because a fill-height child of a box in a list has no bound to fill.
 */
@Composable
fun BoxScope.RightFade(color: androidx.compose.ui.graphics.Color = Mts.Surface000) {
    Box(Modifier.matchParentSize(), contentAlignment = Alignment.CenterEnd) {
        Box(Modifier.width(FADE_DP.dp).fillMaxHeight().background(Brush.horizontalGradient(listOf(androidx.compose.ui.graphics.Color.Transparent, color))))
    }
}

@Composable
fun CodeBlockView(lang: String, text: String, onCopied: () -> Unit) {
    val copy = rememberCopier()
    BlockFrame {
        BlockHeader(HqIcons.Code, lang.ifBlank { "code" }, HeaderLabel, Mts.InkMuted) {
            IconAction(HqIcons.Copy, "Copy code") { copy(text); onCopied() }
        }
        ScrollingMono(text)
    }
}

/** A quote: a 3 dp bar in the line colour, then muted text. */
@Composable
fun QuoteView(content: @Composable () -> Unit) {
    Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min)) {
        Box(Modifier.width(3.dp).fillMaxHeight().background(Mts.Line))
        Box(Modifier.padding(start = 12.dp, top = 2.dp, bottom = 2.dp)) { content() }
    }
}

/** A read-only checklist: ticked boxes are green with a black tick, unticked ones have a line border. */
@Composable
fun ChecklistView(items: List<Pair<Boolean, String>>, onOpenNote: (String) -> Unit) {
    Column {
        items.forEach { (done, label) ->
            Row(Modifier.heightIn(min = 44.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(width = 40.dp, height = 44.dp), contentAlignment = Alignment.Center) {
                    val shape = RoundedCornerShape(6.dp)
                    Box(
                        Modifier.size(22.dp).then(if (done) Modifier.background(Mts.BrandGreen, shape).border(2.dp, Mts.BrandGreen, shape) else Modifier.border(2.dp, Mts.Line, shape)),
                        contentAlignment = Alignment.Center,
                    ) { if (done) Icon(HqIcons.CheckSmall, null, Modifier.size(16.dp), tint = Mts.OnGreen) }
                }
                RichText(label, HqType.Body, Mts.Ink, onOpenNote, Modifier.weight(1f))
            }
        }
    }
}

/** Pictures are not loaded from the internet inside chats; the block says so and offers nothing to tap. */
@Composable
fun BlockedImageView() {
    BlockFrame(border = Mts.Line, dashed = true) {
        Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(HqIcons.Image, null, Modifier.size(24.dp), tint = Mts.InkMuted)
            Column {
                Text("Picture not shown", style = HqType.BodySmBold, color = Mts.Ink)
                Text("HQ does not load pictures from the internet inside chats.", style = HqType.Caption.copy(fontSize = 13.sp, lineHeight = 18.sp), color = Mts.InkMuted)
            }
        }
    }
}

@Composable
fun ListView(ordered: Boolean, items: List<String>, onOpenNote: (String) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        items.forEachIndexed { i, text ->
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(if (ordered) "${i + 1}" else "•", style = HqType.BodyBold, color = Mts.InkMuted, modifier = Modifier.width(20.dp))
                RichText(text, HqType.Body, Mts.Ink, onOpenNote, Modifier.weight(1f))
            }
        }
    }
}
