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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import example.hq.spike.R
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
fun UserBubble(text: String, sending: Boolean, files: List<Attachment> = emptyList(), onOpenFile: (Attachment) -> Unit = {}, modifier: Modifier = Modifier, spoken: Boolean = false) {
    BoxWithConstraints(modifier.fillMaxWidth()) {
        Column(Modifier.align(Alignment.CenterEnd).widthIn(max = maxWidth * USER_BUBBLE_SHARE).alpha(if (sending) 0.7f else 1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (text.isNotBlank()) {
                Text(text, style = HqType.Body, color = Mts.Ink, modifier = Modifier.align(Alignment.End).background(Mts.Surface200, UserShape).padding(horizontal = 16.dp, vertical = 12.dp))
            }
            if (spoken) {
                val byVoice = stringResource(R.string.chat_asked_by_voice)
                Text(stringResource(R.string.chat_spoken), style = HqType.Caption, color = Mts.InkMuted, modifier = Modifier.align(Alignment.End).semantics { contentDescription = byVoice })
            }
            files.forEach { FileCardView(it, { onOpenFile(it) }) }
        }
    }
}

@Composable
private fun ReplyFrame(modifier: Modifier = Modifier, border: androidx.compose.ui.graphics.Color = Mts.Hairline, content: @Composable () -> Unit) {
    Column(
        modifier.fillMaxWidth().clip(ReplyShape).background(Mts.Surface100).border(1.dp, border, ReplyShape)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        content = { content() },
    )
}

@Composable
fun ReplyBubble(message: UiMessage, blocks: BlockActions, lifted: Boolean = false, modifier: Modifier = Modifier, onOffer: ((Boolean) -> Unit)? = null, onScheduleOffer: ((Long) -> Unit)? = null) = ReplyFrame(modifier, if (lifted) Mts.Line else Mts.Hairline) {
    if (message.text.isNotBlank()) ReplyBody(message.text, blocks)
    val cards = remember(message.tools) { StepCards.of(message.tools) }
    message.files.forEach { FileCardView(it, { blocks.onOpenFile(it) }, Modifier.padding(top = 12.dp)) }
    ReplyCards(cards, message.createdAt.let { if (it.isBlank()) "just now" else ChatTime.since(it) }, blocks, Modifier.padding(top = 12.dp))
    if (message.tools.isNotEmpty()) ToolLine(message.tools)
    val offer = message.offer
    if (offer != null && onOffer != null) OfferLine(offer.line, onOffer, onScheduleOffer.takeIf { offer.kind.startsWith(SCHEDULABLE_PREFIX) })
}

/** One thing HQ can also do here, under the reply the person is reading. Not a notification. */
@Composable
private fun OfferLine(line: String, onAnswer: (Boolean) -> Unit, onSchedule: ((Long) -> Unit)?) {
    var picking by remember { mutableStateOf(false) }
    Column(Modifier.padding(top = 12.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(line, style = HqType.BodySm, color = Mts.InkMuted)
        if (picking && onSchedule != null) {
            scheduleChoices().forEach { (label, at) ->
                OfferButton(androidx.compose.ui.res.stringResource(label), Mts.Surface200, Mts.Ink) { onSchedule(at) }
            }
            OfferButton(androidx.compose.ui.res.stringResource(R.string.chat_offer_back), Mts.Surface100, Mts.InkMuted) { picking = false }
        } else {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OfferButton("Yes", Mts.Surface200, Mts.Ink) { onAnswer(true) }
                if (onSchedule != null) OfferButton(androidx.compose.ui.res.stringResource(R.string.chat_offer_schedule), Mts.Surface200, Mts.Ink) { picking = true }
                OfferButton("Not now", Mts.Surface100, Mts.InkMuted) { onAnswer(false) }
            }
        }
    }
}

private const val SCHEDULABLE_PREFIX = "next_"
private const val LATER_TODAY_HOURS = 3L
private const val MORNING_HOUR = 9
private const val NEXT_WEEK_DAYS = 7L

/** What Schedule offers: a label and a time in unix seconds. */
private fun scheduleChoices(now: java.time.ZonedDateTime = java.time.ZonedDateTime.now()): List<Pair<Int, Long>> {
    fun morning(days: Long) = now.plusDays(days).withHour(MORNING_HOUR).withMinute(0).withSecond(0).withNano(0).toEpochSecond()
    return listOf(
        R.string.chat_offer_later_today to now.plusHours(LATER_TODAY_HOURS).toEpochSecond(),
        R.string.chat_offer_tomorrow to morning(1),
        R.string.chat_offer_next_week to morning(NEXT_WEEK_DAYS),
    )
}

@Composable
private fun OfferButton(label: String, fill: androidx.compose.ui.graphics.Color, ink: androidx.compose.ui.graphics.Color, onClick: () -> Unit) {
    Row(
        Modifier.heightIn(min = 48.dp).clip(CircleShape).background(fill).border(1.dp, Mts.Hairline, CircleShape)
            .clickable(role = Role.Button, onClick = onClick).padding(horizontal = 20.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) { Text(label, style = HqType.BodySmBold, color = ink) }
}

/** The collapsed "What I did" line under a finished reply; one tap shows the steps in plain words. */
@Composable
private fun ToolLine(tools: List<ToolStep>) {
    var open by remember { mutableStateOf(false) }
    val lines = remember(tools) { ToolWords.lines(tools) }
    val count = androidx.compose.ui.res.pluralStringResource(R.plurals.chat_steps, lines.size, lines.size)
    Column(Modifier.padding(top = 8.dp)) {
        Row(
            Modifier.fillMaxWidth().minimumInteractiveComponentSize().clickable { open = !open }
                .semantics { role = Role.Button; stateDescription = if (open) "Expanded" else "Collapsed" },
            horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("What I did · $count", style = HqType.ToolLine, color = Mts.InkMuted)
            Icon(HqIcons.ChevronDown, null, Modifier.size(18.dp).rotate(if (open) 180f else 0f), tint = Mts.InkMuted)
        }
        if (open) lines.forEach { Step(it.text(), done = true) }
    }
}

/** A step's sentence, with the note, page or search it named. */
@Composable
private fun StepLine.text(): String = if (arg != null) stringResource(res, arg) else stringResource(res)

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
        Text(live.headline?.text() ?: stringResource(R.string.chat_working), style = HqType.BodySmBold, color = Mts.Ink)
    }
    if (live.steps.isNotEmpty()) {
        Column(
            Modifier.padding(top = 8.dp).drawBehind { drawRect(Mts.Hairline, size = Size(2.dp.toPx(), size.height)) }.padding(start = 6.dp),
        ) { live.steps.forEach { Step(it.line.text(), it.done) } }
    }
    if (live.text.isNotBlank()) ReplyBody(live.text, blocks, Modifier.padding(top = 8.dp), streaming = true)
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
            Text(stringResource(R.string.chat_hq_did_not_finish), style = HqType.BodySm, color = Mts.Ink)
        }
        Row(
            Modifier.heightIn(min = 48.dp).clip(CircleShape).background(Mts.Surface200).clickable(role = Role.Button, onClick = onRetry)
                .padding(horizontal = 24.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(HqIcons.RotateCw, null, Modifier.size(18.dp), tint = Mts.Ink)
            Text(stringResource(R.string.chat_retry), style = HqType.BodySmBold, color = Mts.Ink)
        }
    }
}
