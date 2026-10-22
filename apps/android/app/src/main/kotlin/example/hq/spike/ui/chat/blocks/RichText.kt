package example.hq.spike.ui.chat.blocks

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.ui.text.Placeholder
import androidx.compose.ui.text.PlaceholderVerticalAlign
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import example.hq.spike.ui.shell.HqIcons
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import example.hq.spike.ui.theme.Mts

private val INLINE = Regex("\\*\\*(.+?)\\*\\*|`{1,2}([^`]+)`{1,2}|\\[\\[([^\\]]+)]]|\\[([^\\]]+)]\\(([^)]+)\\)")
private val LinkStyle = TextLinkStyles(SpanStyle(color = Mts.BrandBlue, fontWeight = FontWeight.Bold, textDecoration = TextDecoration.Underline))

/**
 * Inline Markdown for a reply: bold, code, links, `[[wikilinks]]`, and the note references the web app also
 * recognises (a backticked vault path, a link to a `.md` file). A note reference is a blue, bold, underlined
 * link that calls [onOpenNote]; any other link opens in the browser.
 */
fun inlineText(text: String, onOpenNote: (String) -> Unit, onOpenUrl: (String) -> Unit): AnnotatedString = buildAnnotatedString {
    var at = 0
    for (m in INLINE.findAll(text)) {
        append(text.substring(at, m.range.first))
        at = m.range.last + 1
        val bold = m.groups[1]?.value
        val code = m.groups[2]?.value
        val wiki = m.groups[3]?.value
        val label = m.groups[4]?.value
        val href = m.groups[5]?.value
        when {
            bold != null -> withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(bold) }
            code != null -> codeSpan(code, onOpenNote)
            wiki != null -> NoteRefs.wikilink(wiki).let { (target, shown) -> noteLink(NoteRefs.sanitize(target), shown, onOpenNote) }
            else -> externalOrNote(label!!, href!!, onOpenNote, onOpenUrl)
        }
    }
    append(text.substring(at))
}

private fun AnnotatedString.Builder.codeSpan(code: String, onOpenNote: (String) -> Unit) {
    val note = NoteRefs.fromCode(code)
    if (note != null) noteLink(note, code, onOpenNote)
    else withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = Mts.Surface200)) { append(code) }
}

private fun AnnotatedString.Builder.noteLink(path: String?, label: String, onOpenNote: (String) -> Unit) {
    if (path == null) { append(label); return }
    withLink(LinkAnnotation.Clickable("note:$path", LinkStyle) { onOpenNote(path) }) { append(label) }
}

private fun AnnotatedString.Builder.externalOrNote(label: String, href: String, onOpenNote: (String) -> Unit, onOpenUrl: (String) -> Unit) {
    val note = NoteRefs.fromLink(href)
    when {
        note != null -> noteLink(note, label, onOpenNote)
        href.startsWith("http://") || href.startsWith("https://") -> withLink(LinkAnnotation.Clickable("url:$href", LinkStyle) { onOpenUrl(href) }) {
            append(label)
            appendInlineContent(EXTERNAL_ICON, "\u2197")
        }
        else -> append(label)
    }
}

private const val EXTERNAL_ICON = "external"
private val ExternalIcon = mapOf(
    EXTERNAL_ICON to InlineTextContent(Placeholder(17.sp, 14.sp, PlaceholderVerticalAlign.TextCenter)) {
        Icon(HqIcons.External, null, Modifier.padding(start = 3.dp).fillMaxSize(), tint = Mts.BrandBlue)
    },
)

/** A paragraph of reply text with its links live. */
@Composable
fun RichText(text: String, style: TextStyle, color: Color, onOpenNote: (String) -> Unit, modifier: Modifier = Modifier) {
    val uri = LocalUriHandler.current
    val annotated = remember(text, onOpenNote) { inlineText(text, onOpenNote) { uri.openUri(it) } }
    Text(annotated, style = style, color = color, modifier = modifier, inlineContent = ExternalIcon)
}
