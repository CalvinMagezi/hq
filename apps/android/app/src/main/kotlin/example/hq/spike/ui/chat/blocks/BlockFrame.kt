package example.hq.spike.ui.chat.blocks

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

private val FrameShape = RoundedCornerShape(12.dp)
private val DASH = floatArrayOf(6f, 4f)

/** The small bold label a block header uses (12/16, 700). */
val HeaderLabel: TextStyle = HqType.Caption.copy(fontWeight = FontWeight.Bold)

/** The box every rich block sits in: black, 1 dp border, 12 dp corners, clipped. A dashed border marks a block that is not live. */
@Composable
fun BlockFrame(modifier: Modifier = Modifier, border: Color = Mts.Hairline, dashed: Boolean = false, content: @Composable ColumnScope.() -> Unit) {
    val edge = if (dashed) {
        Modifier.drawBehind {
            drawRoundRect(border, cornerRadius = CornerRadius(12.dp.toPx()), style = Stroke(1.dp.toPx(), pathEffect = PathEffect.dashPathEffect(DASH.map { it * density }.toFloatArray())))
        }
    } else {
        Modifier.border(1.dp, border, FrameShape)
    }
    Column(modifier.fillMaxWidth().clip(FrameShape).background(Mts.Surface000).then(edge), content = content)
}

/** Icon, title, and an optional action on the right; 48 dp tall with a hairline under it. */
@Composable
fun BlockHeader(icon: ImageVector, title: String, style: TextStyle, color: Color, action: @Composable () -> Unit = {}) {
    Column {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(start = 12.dp, end = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(icon, null, Modifier.size(18.dp), tint = color)
            Text(title, style = style, color = color, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            action()
        }
        Box(Modifier.fillMaxWidth().height(1.dp).background(Mts.Hairline))
    }
}

/** A 48 dp square button with a 22 dp white icon. */
@Composable
fun IconAction(icon: ImageVector, description: String, iconSize: Dp = 22.dp, onClick: () -> Unit) {
    Box(
        Modifier.size(48.dp).clip(RoundedCornerShape(24.dp)).clickable(role = Role.Button, onClick = onClick).semantics { contentDescription = description },
        contentAlignment = Alignment.Center,
    ) { Icon(icon, null, Modifier.size(iconSize), tint = Mts.Ink) }
}
