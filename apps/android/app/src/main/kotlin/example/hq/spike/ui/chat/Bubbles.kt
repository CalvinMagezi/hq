package example.hq.spike.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.minimumInteractiveComponentSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import example.hq.spike.data.ToolStep
import example.hq.spike.ui.chat.blocks.Attachment
import example.hq.spike.ui.chat.blocks.BlockActions
import example.hq.spike.ui.chat.blocks.FileCardView
import example.hq.spike.ui.chat.blocks.ReplyBody
import example.hq.spike.ui.chat.blocks.ReplyCards
import example.hq.spike.ui.chat.blocks.StepCards
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val UserShape = RoundedCornerShape(topStart = 20.dp, topEnd = 6.dp, bottomEnd = 20.dp, bottomStart = 20.dp)
private val ReplyShape = RoundedCornerShape(topStart = 6.dp, topEnd = 20.dp, bottomEnd = 20.dp, bottomStart = 20.dp)
private const val USER_BUBBLE_SHARE = 0.84f

@Composable
fun UserBubble(text: String, sending: Boolean, files: List<Attachment> = emptyList(), onOpenFile: (Attachment) -> Unit = {}) {
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        Column(Modifier.align(Alignment.CenterEnd).widthIn(max = maxWidth * USER_BUBBLE_SHARE).alpha(if (sending) 0.7f else 1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (text.isNotBlank()) {
                Text(text, style = HqType.Body, color = Mts.Ink, modifier = Modifier.align(Alignment.End).background(Mts.Surface200, UserShape).padding(horizontal = 16.dp, vertical = 12.dp))
            }
            files.forEach { FileCardView(it, { onOpenFile(it) }) }
        }
    }
}

@Composable
private fun ReplyFrame(content: @Composable () -> Unit) {
    Column(
        Modifier.fillMaxWidth().clip(ReplyShape).background(Mts.Surface100).border(1.dp, Mts.Hairline, ReplyShape)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        content = { content() },
    )
}

@Composable
fun ReplyBubble(message: UiMessage, blocks: BlockActions) = ReplyFrame {
    if (message.text.isNotBlank()) ReplyBody(message.text, blocks)
    val cards = remember(message.tools) { StepCards.of(message.tools) }
    message.files.forEach { FileCardView(it, { blocks.onOpenFile(it) }, Modifier.padding(top = 12.dp)) }
    ReplyCards(cards, message.createdAt.let { if (it.isBlank()) "just now" else ChatTime.since(it) }, blocks, Modifier.padding(top = 12.dp))
    if (message.tools.isNotEmpty()) ToolLine(message.tools)
}

/** The collapsed "What I did" line under a finished reply; one tap shows the steps in plain words. */
@Composable
private fun ToolLine(tools: List<ToolStep>) {
    var open by remember { mutableStateOf(false) }
    val count = if (tools.size == 1) "1 step" else "${tools.size} steps"
    Column(Modifier.padding(top = 8.dp)) {
        Row(
            Modifier.fillMaxWidth().minimumInteractiveComponentSize().clickable { open = !open }
                .semantics { role = Role.Button; stateDescription = if (open) "Expanded" else "Collapsed" },
            horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("What I did · $count", style = HqType.ToolLine, color = Mts.InkMuted)
            Icon(HqIcons.ChevronDown, null, Modifier.size(18.dp).rotate(if (open) 180f else 0f), tint = Mts.InkMuted)
        }
        if (open) tools.forEach { Step(ToolWords.running(it), done = true) }
    }
}

@Composable
private fun Step(label: String, done: Boolean) {
    Row(Modifier.heightIn(min = 32.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
        if (done) Icon(HqIcons.Check, null, Modifier.size(16.dp), tint = Mts.BrandGreen) else Spinner(16)
        Text(label, style = HqType.BodySm, color = Mts.InkMuted)
    }
}

@Composable
fun Spinner(size: Int, color: androidx.compose.ui.graphics.Color = Mts.BrandBlue) {
    CircularProgressIndicator(Modifier.size(size.dp), color = color, strokeWidth = 2.dp)
}

/** The reply being written: what HQ is doing, its steps so far, and the text as it arrives. */
@Composable
fun LiveReplyBubble(live: LiveReply, blocks: BlockActions) = ReplyFrame {
    Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
        Spinner(18)
        Text(live.headline, style = HqType.BodySmBold, color = Mts.Ink)
    }
    if (live.steps.isNotEmpty()) {
        Column(
            Modifier.padding(top = 8.dp).drawBehind { drawRect(Mts.Hairline, size = Size(2.dp.toPx(), size.height)) }.padding(start = 6.dp),
        ) { live.steps.forEach { Step(it.label, it.done) } }
    }
    if (live.text.isNotBlank()) ReplyBody(live.text, blocks, Modifier.padding(top = 8.dp))
}

@Composable
fun FailedCard(onRetry: () -> Unit) {
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Mts.Surface100)
            .border(1.dp, Mts.Hairline, RoundedCornerShape(12.dp)).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(HqIcons.Warning, null, Modifier.size(20.dp), tint = Mts.Amber)
            Text("HQ did not finish a reply to this message.", style = HqType.BodySm, color = Mts.Ink)
        }
        Row(
            Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).clickable(role = Role.Button, onClick = onRetry)
                .padding(horizontal = 24.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(HqIcons.Redo, null, Modifier.size(18.dp), tint = Mts.Ink)
            Text("Retry", style = HqType.BodySmBold, color = Mts.Ink)
        }
    }
}
