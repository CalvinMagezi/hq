package example.hq.spike.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import example.hq.spike.R
import example.hq.spike.data.Offer
import example.hq.spike.ui.shell.AttentionGate
import example.hq.spike.ui.shell.AttentionKind
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val SlotShape = RoundedCornerShape(16.dp)
private const val RESULT_KEY = "result"
private const val LATER_TODAY_HOURS = 3L
private const val MORNING_HOUR = 9
private const val NEXT_WEEK_DAYS = 7L

/**
 * The offer in the one attention slot above the composer: one question with Yes, Not now and Later, then one line saying what
 * was done. The caller shows it only when no approval waits, so two things never compete here.
 */
@Composable
fun OfferSlot(state: ChatUiState, actions: ChatActions) {
    val message = state.offered
    val offer = message?.offer
    when {
        offer != null -> AttentionGate(AttentionKind.Offer, message.id) { OfferCard(offer, state.laterSupported, { actions.onOffer(message.id, it) }, { at -> actions.onScheduleOffer(message.id, at) }) }
        state.offerResult != null -> AttentionGate(AttentionKind.Offer, RESULT_KEY) { ResultLine(state.offerResult) }
    }
}

@Composable
private fun OfferCard(offer: Offer, laterSupported: Boolean, onAnswer: (OfferAnswer) -> Unit, onSchedule: (Long) -> Unit) {
    var picking by rememberSaveable { mutableStateOf(false) }
    val stacked = SlotLayout.stacked(LocalConfiguration.current.fontScale)
    val canSchedule = ChatSlot.schedulable(offer, laterSupported)
    Column(
        Modifier.padding(horizontal = 16.dp, vertical = 8.dp).fillMaxWidth().clip(SlotShape).background(Mts.Surface100).border(1.dp, Mts.Hairline, SlotShape).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        // A polite live region: TalkBack reads the question when it appears, without taking focus.
        Text(offer.line, style = HqType.BodySm, color = Mts.Ink, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
        if (picking) {
            Buttons(stacked) { m ->
                scheduleChoices().forEach { (label, at) -> OfferButton(stringResource(label), Mts.Surface200, Mts.Ink, m) { onSchedule(at) } }
                OfferButton(stringResource(R.string.chat_offer_back), Mts.Surface100, Mts.InkMuted, m) { picking = false }
            }
        } else {
            Buttons(stacked) { m ->
                ChatSlot.answers(offer, laterSupported).forEach { answer ->
                    val (label, fill, ink) = when (answer) {
                        OfferAnswer.Yes -> Triple(R.string.chat_offer_yes, Mts.BrandGreen, Mts.OnGreen)
                        OfferAnswer.NotNow -> Triple(R.string.chat_offer_not_now, Mts.Surface200, Mts.Ink)
                        OfferAnswer.Later -> Triple(R.string.chat_offer_later, Mts.Surface200, Mts.Ink)
                    }
                    OfferButton(stringResource(label), fill, ink, m) { onAnswer(answer) }
                }
                if (canSchedule) OfferButton(stringResource(R.string.chat_offer_schedule), Mts.Surface200, Mts.Ink, m) { picking = true }
            }
        }
    }
}

/** Buttons side by side, or one under another from [SlotLayout.STACK_FROM]; each is handed the modifier that sizes it. */
@Composable
private fun Buttons(stacked: Boolean, content: @Composable (Modifier) -> Unit) {
    if (stacked) {
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) { content(Modifier.fillMaxWidth()) }
    } else {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) { content(Modifier) }
    }
}

/** What an answer left behind: one line, read out politely, gone by itself. */
@Composable
private fun ResultLine(result: OfferResult) {
    val words = stringResource(if (result == OfferResult.Later) R.string.chat_offer_result_later else R.string.chat_offer_result_not_now)
    Text(
        words, style = HqType.BodySm, color = Mts.InkMuted,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp).fillMaxWidth().heightIn(min = 20.dp).semantics { liveRegion = LiveRegionMode.Polite },
    )
}

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
private fun OfferButton(label: String, fill: Color, ink: Color, modifier: Modifier = Modifier, onClick: () -> Unit) {
    Row(
        modifier.heightIn(min = 48.dp).clip(CircleShape).background(fill).border(1.dp, Mts.Hairline, CircleShape)
            .clickable(role = Role.Button, onClick = onClick).padding(horizontal = 20.dp),
        horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically,
    ) { Text(label, style = HqType.BodySmBold, color = ink) }
}
