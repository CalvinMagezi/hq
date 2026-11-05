package example.hq.spike.ui.startup

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import kotlin.math.sin
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.R
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val Title = HqType.H2.copy(fontSize = 28.sp, lineHeight = 34.sp)

/** One 4 dp bar filled to the steps done, with the four steps listed under it; the current one pulses, also under reduced motion. */
@Composable
fun StartSteps(step: Int, seconds: Long, modifier: Modifier = Modifier, showCount: Boolean = true) {
    val done = (step - 1).coerceIn(0, StartRules.STEPS)
    // Driven by frame time, not an animator, so it still pulses when the phone asks for less motion (that setting zeroes animators).
    var pulse by remember { mutableFloatStateOf(1f) }
    LaunchedEffect(Unit) { while (true) withFrameNanos { pulse = PULSE_LOW + (1f - PULSE_LOW) * (0.5f + 0.5f * sin(it / NANOS * 2.0 * Math.PI / PULSE_SECONDS).toFloat()) } }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(Modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)).background(Mts.Surface200)) {
            if (done > 0) Box(Modifier.weight(done.toFloat()).fillMaxHeight().background(Mts.BrandBlue))
            if (done < StartRules.STEPS) Spacer(Modifier.weight((StartRules.STEPS - done).toFloat()))
        }
        for (n in 1..StartRules.STEPS) {
            val name = stringResource(RunningNotice.stepName(n))
            val label = stringResource(R.string.start_step_talkback, n, name)
            val current = n == step
            Row(Modifier.heightIn(min = 28.dp).clearAndSetSemantics { contentDescription = label }, horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                if (n < step) Icon(HqIcons.Check, null, Modifier.size(18.dp), tint = Mts.BrandGreen)
                else Box(Modifier.size(18.dp).padding(5.dp).background(if (current) Mts.BrandBlue.copy(alpha = pulse) else Mts.Line, CircleShape))
                Text(name, style = if (current) HqType.BodySmBold else HqType.BodySm, color = if (n > step) Mts.InkMuted else Mts.Ink)
            }
        }
        if (showCount) Text(stringResource(R.string.start_count, step, seconds), style = HqType.Caption, color = Mts.InkMuted)
    }
}

private const val PULSE_LOW = 0.35f
private const val PULSE_SECONDS = 1.8
private const val NANOS = 1e9

/** The first start: nothing is cached yet, so a screen that says HQ is starting, the four steps, and that a thought can still be saved. */
@Composable
fun StartingScreen(step: Int, seconds: Long, onCapture: () -> Unit) {
    Column(
        Modifier.fillMaxSize().background(Mts.Surface000).padding(horizontal = 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Image(painterResource(R.drawable.hq_mark), null, Modifier.size(88.dp))
        Text(stringResource(R.string.start_title), style = Title, color = Mts.Ink, textAlign = TextAlign.Center)
        Text(stringResource(R.string.start_body), style = HqType.Body, color = Mts.InkMuted, textAlign = TextAlign.Center)
        StartSteps(step, seconds, Modifier.widthIn(max = 320.dp))
        Column(Modifier.padding(top = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Text(stringResource(R.string.start_capture_hint), style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.Center)
            Row(
                Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClick = onCapture).padding(horizontal = 24.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(HqIcons.Plus, null, Modifier.size(20.dp), tint = Mts.Ink)
                Text(stringResource(R.string.start_capture), style = HqType.BodySmBold, color = Mts.Ink)
            }
        }
    }
}
