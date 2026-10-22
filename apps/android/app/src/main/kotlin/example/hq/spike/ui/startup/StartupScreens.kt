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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.R
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private const val PROGRESS_FILL = 0.4f
private val Title = HqType.H2.copy(fontSize = 28.sp, lineHeight = 34.sp)

/** The first start: nothing is cached yet, so a screen that says HQ is starting, for how long, and that a thought can still be saved. */
@Composable
fun StartingScreen(seconds: Long, onCapture: () -> Unit) {
    Column(
        Modifier.fillMaxSize().background(Mts.Surface000).padding(horizontal = 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Image(painterResource(R.drawable.hq_mark), null, Modifier.size(88.dp))
        Text(StartupWords.STARTING_TITLE, style = Title, color = Mts.Ink, textAlign = TextAlign.Center)
        Text(StartupWords.STARTING_BODY, style = HqType.Body, color = Mts.InkMuted, textAlign = TextAlign.Center)
        Box(Modifier.width(220.dp).heightIn(min = 4.dp).clip(RoundedCornerShape(2.dp)).background(Mts.Surface200)) {
            Box(Modifier.width(220.dp * PROGRESS_FILL).heightIn(min = 4.dp).clip(RoundedCornerShape(2.dp)).background(Mts.BrandBlue))
        }
        Text("$seconds s", style = HqType.Caption, color = Mts.InkMuted)
        Column(Modifier.padding(top = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Text(StartupWords.CAPTURE_HINT, style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.Center)
            Row(
                Modifier.heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClick = onCapture).padding(horizontal = 24.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(HqIcons.Plus, null, Modifier.size(20.dp), tint = Mts.Ink)
                Text("Capture", style = HqType.BodySmBold, color = Mts.Ink)
            }
        }
    }
}

/** After a minute with no answer: the likely causes, what is known about the last start, and ways to try again. */
@Composable
fun DidNotStartScreen(lines: List<String>, onTryAgain: () -> Unit, onBattery: () -> Unit, onCopy: () -> Unit) {
    Column(
        Modifier.fillMaxSize().background(Mts.Surface000).padding(horizontal = 32.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(HqIcons.Warning, null, Modifier.size(48.dp), tint = Mts.Amber)
        Text(StartupWords.FAILED_TITLE, style = Title, color = Mts.Ink, textAlign = TextAlign.Center)
        Text(StartupWords.FAILED_BODY, style = HqType.Body, color = Mts.InkMuted, textAlign = TextAlign.Center)
        val shape = RoundedCornerShape(8.dp)
        Column(Modifier.fillMaxWidth().clip(shape).background(Mts.Surface000).border(1.dp, Mts.Hairline, shape).padding(12.dp)) {
            lines.forEach { Text(it, style = HqType.Caption.copy(fontFamily = FontFamily.Monospace, fontSize = 12.sp, lineHeight = 16.sp), color = Mts.InkMuted) }
        }
        Column(Modifier.padding(top = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Box(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button, onClick = onTryAgain), contentAlignment = Alignment.Center) {
                Text("Try again", style = HqType.BodySmBold, color = Mts.OnGreen)
            }
            Box(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(role = Role.Button, onClick = onBattery), contentAlignment = Alignment.Center) {
                Text("Battery settings", style = HqType.BodySmBold, color = Mts.Ink)
            }
            Box(Modifier.heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onCopy).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
                Text("Copy details", style = HqType.BodySmBold, color = Mts.BrandBlue)
            }
        }
    }
}
