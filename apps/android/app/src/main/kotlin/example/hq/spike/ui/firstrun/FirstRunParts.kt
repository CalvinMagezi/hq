package example.hq.spike.ui.firstrun

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import example.hq.spike.ui.chat.Spinner
import example.hq.spike.ui.chat.blocks.IconAction
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.theme.HqType
import example.hq.spike.ui.theme.Mts

/** A full-width green pill. Grey while it cannot be used; a spinner while [busy]. */
@Composable
fun GreenPill(text: String, enabled: Boolean = true, busy: Boolean = false, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(CircleShape).background(if (enabled || busy) Mts.BrandGreen else Mts.Surface200).clickable(enabled = enabled && !busy, role = Role.Button, onClick = onClick),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically,
    ) {
        if (busy) Spinner(18, Mts.OnGreen)
        Text(text, style = HqType.BodySmBold, color = if (enabled || busy) Mts.OnGreen else Mts.InkMuted)
    }
}

/** A full-width outlined pill, for the second choice on a screen. */
@Composable
fun OutlinePill(text: String, enabled: Boolean = true, onClick: () -> Unit) {
    Box(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(CircleShape).border(1.dp, Mts.Line, CircleShape).clickable(enabled = enabled, role = Role.Button, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { Text(text, style = HqType.BodySmBold, color = if (enabled) Mts.Ink else Mts.InkMuted) }
}

/** A text-only action, such as Back or Skip. */
@Composable
fun TextAction(text: String, enabled: Boolean = true, onClick: () -> Unit) {
    Box(Modifier.heightIn(min = 48.dp).clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(horizontal = 12.dp), contentAlignment = Alignment.Center) {
        Text(text, style = HqType.BodySmBold, color = if (enabled) Mts.BrandBlue else Mts.InkMuted)
    }
}

/** One labelled line of plain text, with a hint while it is empty. */
@Composable
fun PlainField(label: String, value: String, hint: String, enabled: Boolean = true, keyboard: KeyboardType = KeyboardType.Text, onChange: (String) -> Unit) {
    val field = RoundedCornerShape(12.dp)
    Column(Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(field).background(Mts.Surface000).border(2.dp, Mts.Line, field).padding(horizontal = 16.dp, vertical = 8.dp)) {
        Text(label, style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = Mts.InkMuted)
        Box {
            if (value.isEmpty()) Text(hint, style = HqType.Body, color = Mts.InkMuted)
            BasicTextField(
                value, onChange, Modifier.fillMaxWidth(), enabled = enabled, singleLine = true,
                textStyle = HqType.Body.copy(color = Mts.Ink), cursorBrush = SolidColor(Mts.Ink),
                keyboardOptions = KeyboardOptions(keyboardType = keyboard, autoCorrectEnabled = false),
            )
        }
    }
}

/**
 * A secret line: dots unless [show], an eye to reveal it, and an optional Paste. The text lives only in the caller's
 * state; it is never saved by the field.
 */
@Composable
fun SecretField(
    label: String, value: String, hint: String, show: Boolean, enabled: Boolean = true, bad: Boolean = false,
    onShow: () -> Unit, onChange: (String) -> Unit,
) {
    val clipboard = LocalClipboardManager.current
    val field = RoundedCornerShape(12.dp)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).clip(field).background(Mts.Surface000).border(2.dp, if (bad) Mts.DangerText else Mts.Line, field).padding(start = 16.dp, top = 8.dp, end = 8.dp, bottom = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(label, style = HqType.Caption.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Bold), color = if (bad) Mts.DangerText else Mts.InkMuted)
            Box {
                if (value.isEmpty()) Text(hint, style = HqType.Body, color = Mts.InkMuted)
                BasicTextField(
                    value, onChange, Modifier.fillMaxWidth(), enabled = enabled, singleLine = true,
                    textStyle = HqType.Body.copy(color = Mts.Ink, fontFamily = FontFamily.Monospace, letterSpacing = 0.1.em), cursorBrush = SolidColor(Mts.Ink),
                    visualTransformation = if (show) VisualTransformation.None else PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
                )
            }
        }
        IconAction(HqIcons.Paste, FirstRunWords.PASTE, iconSize = 22.dp) { clipboard.getText()?.text?.let { onChange(it.trim()) } }
        IconAction(if (show) HqIcons.EyeOff else HqIcons.Eye, if (show) FirstRunWords.HIDE else FirstRunWords.SHOW, iconSize = 22.dp, onClick = onShow)
    }
}

/** An error or result line under a field. */
@Composable
fun Message(text: String, good: Boolean = false) {
    Text(text, style = HqType.BodySm, color = if (good) Mts.BrandGreen else Mts.DangerText, textAlign = TextAlign.Start, modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp))
}
