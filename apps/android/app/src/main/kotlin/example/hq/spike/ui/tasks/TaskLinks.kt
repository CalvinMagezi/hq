package example.hq.spike.ui.tasks

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import example.hq.spike.ui.theme.Mts

/** A task's description with its web addresses made into links that open in the browser. */
object TaskLinks {
    private val URL = Regex("https?://[^\\s<>\"']+")
    private const val SENTENCE_END = ".,;:!?)]}"
    private val Style = TextLinkStyles(SpanStyle(color = Mts.BrandBlue, textDecoration = TextDecoration.Underline))

    fun annotated(text: String): AnnotatedString = buildAnnotatedString {
        var at = 0
        for (m in URL.findAll(text)) {
            // A full stop or bracket right after an address belongs to the sentence, not to the address.
            val url = m.value.trimEnd { it in SENTENCE_END }
            append(text.substring(at, m.range.first))
            withLink(LinkAnnotation.Url(url, Style)) { append(url) }
            at = m.range.first + url.length
        }
        append(text.substring(at))
    }
}
