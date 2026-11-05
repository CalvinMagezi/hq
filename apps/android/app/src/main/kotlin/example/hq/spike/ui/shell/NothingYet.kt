package example.hq.spike.ui.shell

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** The page a screen shows while HQ cannot answer it: what opens when HQ is up, why, and the one button that helps, if any. */
@Composable
fun NothingYet(title: String, body: String, action: String? = null, onAction: () -> Unit = {}) {
    Column(Modifier.fillMaxSize().background(Mts.Surface000).padding(32.dp), verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(title, style = HqType.H2, color = Mts.Ink, textAlign = TextAlign.Center)
        if (body.isNotEmpty()) Text(body, style = HqType.BodySm, color = Mts.InkMuted, textAlign = TextAlign.Center)
        if (action != null) Box(Modifier.padding(top = 8.dp).heightIn(min = 48.dp).clip(CircleShape).background(Mts.BrandGreen).clickable(role = Role.Button, onClick = onAction).padding(horizontal = 32.dp), contentAlignment = Alignment.Center) {
            Text(action, style = HqType.BodySmBold, color = Mts.OnGreen)
        }
    }
}
