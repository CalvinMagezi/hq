package example.hq.spike.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.unit.sp
import example.hq.spike.R

/** The MTS dark tokens from the design (hq-ui.css), by the design's own names. */
object Mts {
    val Surface000 = Color(0xFF000000)
    val Surface100 = Color(0xFF231F20)
    val Surface200 = Color(0xFF322E2F)
    val Ink = Color(0xFFFFFFFF)
    val InkMuted = Color(0xFFD0D2D3)
    val Line = Color(0xFF8A8687)
    val Hairline = Color(0xFF4A4647)
    val BrandBlue = Color(0xFF00ADEE)
    val BrandGreen = Color(0xFF22FF37)
    val OnGreen = Color(0xFF000000)
    val Amber = Color(0xFFFFB300)
    val DangerText = Color(0xFFFF6B6B)
    val LauncherGround = Color(0xFF0A0A0F)
    val BrandIndigo = Color(0xFF2E3191)
    /** Undo on the snackbar; the design uses it outside the MTS set. */
    val UndoBlue = Color(0xFF00668F)
}

val ClearSans = FontFamily(
    Font(R.font.clearsans_regular, FontWeight.Normal),
    Font(R.font.clearsans_bold, FontWeight.Bold),
)

private val CenteredLine = LineHeightStyle(LineHeightStyle.Alignment.Center, LineHeightStyle.Trim.None)

private fun style(size: Int, line: Int, weight: FontWeight) = TextStyle(
    fontFamily = ClearSans, fontSize = size.sp, lineHeight = line.sp, fontWeight = weight, lineHeightStyle = CenteredLine,
)

/** Text styles named for where the design uses them, written as size / line height. */
object HqType {
    val Body = style(16, 24, FontWeight.Normal)
    val BodySm = style(14, 20, FontWeight.Normal)
    val BodyBold = style(16, 24, FontWeight.Bold)
    val Caption = style(12, 16, FontWeight.Normal)
    val ScreenTitle = style(18, 24, FontWeight.Bold)
    val NavLabel = style(12, 16, FontWeight.Bold)
    val Badge = style(11, 16, FontWeight.Bold)
    val BodySmBold = style(14, 20, FontWeight.Bold)
    val ToolLine = style(13, 16, FontWeight.Bold)
    val H2 = style(22, 28, FontWeight.Bold)
    val ChipTitle = style(15, 20, FontWeight.Bold)
    val Group = style(13, 16, FontWeight.Bold)
}

@Composable
fun HqTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(
            primary = Mts.BrandGreen, onPrimary = Mts.OnGreen,
            background = Mts.Surface000, onBackground = Mts.Ink,
            surface = Mts.Surface100, onSurface = Mts.Ink,
            surfaceVariant = Mts.Surface200, onSurfaceVariant = Mts.InkMuted,
            outline = Mts.Line, outlineVariant = Mts.Hairline, error = Mts.DangerText,
        ),
        content = content,
    )
}
