package example.hq.spike.ui.chat.blocks

import androidx.compose.ui.text.LinkAnnotation
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class RichTextTest {
    private val path = "Notebooks/Projects/Agent-HQ/Remote-Coding-Agent-Sandboxes-Assessment.md"

    private fun links(text: String): Pair<String, List<String>> {
        val opened = mutableListOf<String>()
        val a = inlineText(text, { opened += it }, {})
        return a.text to a.getLinkAnnotations(0, a.length).map { (it.item as LinkAnnotation.Clickable).tag }
    }

    @Test fun aSingleTickNotePathIsALinkWithNoTicks() {
        val (text, tags) = links("saved in `$path` today")
        assertEquals("saved in $path today", text)
        assertEquals(listOf("note:$path"), tags)
    }

    @Test fun aDoubleTickNotePathIsALinkWithNoTicksAroundIt() {
        val (text, tags) = links("saved in ``$path`` today")
        assertFalse(text.contains('`'))
        assertEquals("saved in $path today", text)
        assertEquals(listOf("note:$path"), tags)
    }

    @Test fun otherCodeStaysCodeAndEveryTickIsConsumed() {
        val (text, tags) = links("run ``make test`` or `make`, then ``a`` `b`")
        assertEquals("run make test or make, then a b", text)
        assertEquals(emptyList<String>(), tags)
    }

    @Test fun anEventLinkIsLiveOnlyInItsExactShape() {
        val (_, good) = links("see [Dentist](hq://calendar/event/7?begin=1&end=2)")
        assertEquals(listOf("calendar:7"), good)
        val (text, bad) = links("see [Dentist](hq://calendar/event/7?begin=1&end=2&x=3) and [x](content://com.android.calendar/events/7)")
        assertEquals("see Dentist and x", text)
        assertEquals(emptyList<String>(), bad)
    }
}
