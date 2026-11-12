package example.hq.spike.ui.notes

import example.hq.spike.ui.chat.NoteMenuActions
import example.hq.spike.ui.chat.NoteMenuItems
import example.hq.spike.ui.chat.NoteSheet
import example.hq.spike.ui.shell.HqIcons
import example.hq.spike.ui.shell.SheetAction
import example.hq.spike.ui.shell.SheetGroup
import example.hq.spike.ui.shell.SheetItem
import example.hq.spike.ui.shell.SheetList
import org.junit.Test
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue

class NoteMenuItemsTest {
    private val words = NoteMenuItems.Words("Edit", "Pin", "Unpin", "Ask HQ", "Make a task", "Share and export", "Share", "Export as…", "hint", "Copy path", "Copy note text", "Delete", "More", "Too long")
    private val sheet = NoteSheet("Notebooks/a.md", "A", "", "body", false, null, pinned = false)
    private fun build(a: NoteMenuActions, pinned: Boolean = false) = NoteMenuItems.build(sheet.copy(pinned = pinned), a, words, {}, {}, {})
    private fun labels(items: List<SheetItem>) = items.map { it.label }

    @Test fun `the reader sheet holds pin, make a task, share and export, delete`() {
        val items = build(NoteMenuActions(onAsk = null, onMakeTask = {}, onDelete = {}))
        assertEquals(listOf("Pin", "Make a task", "Share and export", "Delete"), labels(items))
    }

    @Test fun `share, export and both copies live in one group`() {
        val group = build(NoteMenuActions(onAsk = null, onDelete = {})).filterIsInstance<SheetGroup>().single()
        assertEquals(listOf("Share", "Export as…", "Copy note text", "Copy path"), group.items.map { it.label })
    }

    @Test fun `the chat sheet keeps Ask HQ and has no delete`() {
        val items = build(NoteMenuActions())
        assertEquals(listOf("Pin", "Ask HQ", "Share and export"), labels(items))
    }

    @Test fun `a pinned note offers Unpin`() {
        assertEquals("Unpin", build(NoteMenuActions(onAsk = null), pinned = true).first().label)
    }

    private fun leaves(items: List<SheetItem>): List<String> = items.flatMap { if (it is SheetGroup) it.items.map { a -> a.label } else listOf(it.label) }

    @Test fun `no sheet is longer than six rows and none is lost`() {
        val everything = build(NoteMenuActions(onEdit = {}, onAsk = {}, onMakeTask = {}, onDelete = {}))
        assertTrue(everything.size <= SheetList.MAX)
        assertEquals(setOf("Edit", "Pin", "Ask HQ", "Make a task", "Share", "Export as…", "Copy note text", "Copy path", "Delete"), leaves(everything).toSet())
    }

    @Test fun `fit never drops a row, for any length`() {
        for (n in 0..20) {
            val rows = (1..n).map { SheetAction(HqIcons.Pin, "Row $it") {} }
            val fitted = SheetList.fit(rows, HqIcons.MoreVertical, "More")
            assertTrue("n=$n gave ${fitted.size} rows", fitted.size <= SheetList.MAX)
            assertEquals("n=$n lost a row", rows.map { it.label }, leaves(fitted).filter { it != "More" })
            if (n > SheetList.MAX) assertTrue("n=$n has no More group", fitted.last() is SheetGroup)
        }
    }

    @Test fun `sharing a truncated note says it is too long and shares nothing`() {
        var shared = 0
        val said = mutableListOf<String>()
        val items = NoteMenuItems.build(sheet.copy(truncated = true), NoteMenuActions(onAsk = null, onNotice = { said += it }), words, { shared++ }, {}, {})
        val share = (items.filterIsInstance<SheetGroup>().single().items).first { it.label == "Share" }
        share.onClick()
        assertEquals(0, shared)
        assertEquals(listOf("Too long"), said)
    }

    @Test fun `sharing a whole note shares it`() {
        var shared = 0
        val items = NoteMenuItems.build(sheet, NoteMenuActions(onAsk = null), words, { shared++ }, {}, {})
        items.filterIsInstance<SheetGroup>().single().items.first { it.label == "Share" }.onClick()
        assertEquals(1, shared)
    }

    @Test fun `extra rows fold into a More group`() {
        val rows = (1..9).map { SheetAction(HqIcons.Pin, "Row $it") {} }
        val more = SheetList.fit(rows, HqIcons.MoreVertical, "More").last() as SheetGroup
        assertEquals((6..9).map { "Row $it" }, more.items.map { it.label })
    }

    @Test fun `six rows or fewer are left alone`() {
        val rows = (1..6).map { SheetAction(HqIcons.Pin, "Row $it") {} }
        assertEquals(rows, SheetList.fit(rows, HqIcons.MoreVertical, "More"))
    }

    @Test fun `a group in the overflow is flattened into More`() {
        val rows: List<SheetItem> = (1..5).map { SheetAction(HqIcons.Pin, "Row $it") {} } + listOf(SheetGroup(HqIcons.Share, "G", listOf(SheetAction(HqIcons.Share, "Inner") {}))) + SheetAction(HqIcons.Pin, "Last") {}
        val more = SheetList.fit(rows, HqIcons.MoreVertical, "More").last() as SheetGroup
        assertEquals(listOf("Inner", "Last"), more.items.map { it.label })
    }
}
