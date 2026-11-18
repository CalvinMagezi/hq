package example.hq.spike.ui.notes

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import org.junit.Assert.assertEquals
import org.junit.Test

class MarkdownFormatTest {
    private fun v(text: String, start: Int, end: Int = start) = TextFieldValue(text, TextRange(start, end))

    @Test
    fun boldWrapsTheSelectionAndKeepsItSelected() {
        val r = v("make this loud", 5, 9).bold()
        assertEquals("make **this** loud", r.text)
        assertEquals(TextRange(7, 11), r.selection)
    }

    @Test
    fun boldAgainTakesTheMarkersOff() {
        val r = v("make **this** loud", 7, 11).bold()
        assertEquals("make this loud", r.text)
        assertEquals(TextRange(5, 9), r.selection)
    }

    @Test
    fun withNothingSelectedAnEmptyPairIsAddedAndTheCursorSitsBetween() {
        val r = v("ab", 1).code()
        assertEquals("a``b", r.text)
        assertEquals(TextRange(2), r.selection)
    }

    @Test
    fun italicInsideBoldWrapsInsteadOfUnwrapping() {
        val r = v("**this**", 2, 6).italic()
        assertEquals("***this***", r.text)
    }

    @Test
    fun italicAroundTheSelectionComesOff() {
        assertEquals("this", v("*this*", 1, 5).italic().text)
    }

    @Test
    fun aLinkKeepsTheSelectedTextAndSelectsTheAddress() {
        val r = v("see the docs now", 8, 12).link()
        assertEquals("see the [docs](https://) now", r.text)
        assertEquals("https://", r.text.substring(r.selection.min, r.selection.max))
    }

    @Test
    fun aLinkWithNothingSelectedGetsPlaceholderText() {
        val r = v("", 0).link()
        assertEquals("[text](https://)", r.text)
        assertEquals("https://", r.text.substring(r.selection.min, r.selection.max))
    }

    @Test
    fun aHeadingIsSetSwappedAndRemoved() {
        val h2 = v("Title", 2).toggleLines(LineStyle.Heading(2))
        assertEquals("## Title", h2.text)
        assertEquals("# Title", h2.toggleLines(LineStyle.Heading(1)).text)
        assertEquals("Title", h2.toggleLines(LineStyle.Heading(2)).text)
    }

    @Test
    fun aBulletListCoversEveryLineTheSelectionTouchesAndOnlyThose() {
        val r = v("one\ntwo\nthree\nfour", 5, 9).toggleLines(LineStyle.Bullet)
        assertEquals("one\n- two\n- three\nfour", r.text)
        assertEquals("one\ntwo\nthree\nfour", r.toggleLines(LineStyle.Bullet).text)
    }

    @Test
    fun aNumberedListCountsUpAndSwapsFromBullets() {
        val bullets = v("a\nb\nc", 0, 5).toggleLines(LineStyle.Bullet)
        assertEquals("1. a\n2. b\n3. c", bullets.toggleLines(LineStyle.Numbered).text)
    }

    @Test
    fun aQuoteGoesOnTheCursorLineEvenAtTheStartAndEndOfTheText() {
        assertEquals("> hello", v("hello", 0).toggleLines(LineStyle.Quote).text)
        assertEquals("x\n> hello", v("x\nhello", 7).toggleLines(LineStyle.Quote).text)
    }

    @Test
    fun aMixedSelectionIsSetNotCleared() {
        val r = v("- a\nb", 0, 5).toggleLines(LineStyle.Bullet)
        assertEquals("- a\n- b", r.text)
    }
}
