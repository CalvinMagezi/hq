package example.hq.spike.ui.notes

import example.hq.spike.data.HqClient.NoteItem
import org.junit.Test
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue

class VaultRulesTest {
    private fun note(path: String, writtenBy: String = "you", review: String? = null) = NoteItem(path, "", "", "", false, writtenBy, review)

    @Test fun `six folders show and the rest sit behind More`() {
        val c = VaultRules.chips((1..9).map { "F$it" }, 0)
        assertEquals((1..6).map { "F$it" }, c.shown)
        assertEquals(listOf("F7", "F8", "F9"), c.more)
        assertTrue(c.hasMore)
    }

    @Test fun `six or fewer folders have no More chip`() {
        assertFalse(VaultRules.chips(listOf("Inbox", "Projects"), 0).hasMore)
        assertFalse(VaultRules.chips((1..6).map { "F$it" }, 0).hasMore)
    }

    @Test fun `the Trash count never goes below zero`() {
        assertEquals(0, VaultRules.chips(emptyList(), -3).trash)
        assertEquals(4, VaultRules.chips(emptyList(), 4).trash)
    }

    @Test fun `pinned notes stop at six`() {
        assertEquals(6, VaultRules.pinnedShown((1..10).map { note("Notebooks/n$it.md") }).size)
        assertEquals(0, VaultRules.pinnedShown(emptyList()).size)
    }

    @Test fun `review words are New and Changed only`() {
        assertEquals(ReviewMark.New, VaultRules.markOf("new"))
        assertEquals(ReviewMark.Changed, VaultRules.markOf("changed"))
        assertEquals(ReviewMark.None, VaultRules.markOf(null))
        assertEquals(ReviewMark.None, VaultRules.markOf("accepted"))
    }

    @Test fun `only an HQ note shows the HQ word`() {
        assertTrue(VaultRules.writtenByHq(note("a.md", "hq")))
        assertFalse(VaultRules.writtenByHq(note("a.md", "you")))
        assertFalse(VaultRules.writtenByHq(note("a.md", "import")))
    }

    @Test fun `the task is named from the front matter title`() {
        val text = "---\ntitle: \"Mobile roadmap\"\ntags: [a]\n---\n# Something else\nbody"
        assertEquals("Mobile roadmap", VaultRules.titleOf(text, "Notebooks/Projects/2026-11-10-roadmap.md"))
    }

    @Test fun `without a title the first heading names it`() {
        assertEquals("Q3 review", VaultRules.titleOf("---\ntags: [a]\n---\n\n# Q3 review\ntext", "Notebooks/x/q3-review-final.md"))
        assertEquals("Q3 review", VaultRules.titleOf("# Q3 review\ntext", "Notebooks/x/q3.md"))
    }

    @Test fun `with neither the file name is the last resort`() {
        assertEquals("backup-setup", VaultRules.titleOf("just text", "Notebooks/Projects/backup-setup.md"))
        assertEquals("backup-setup", VaultRules.titleOf("---\nstatus: open\n---\ntext", "Notebooks/Projects/backup-setup.md"))
    }

    @Test fun `the task body carries the note title not the file name`() {
        val title = VaultRules.titleOf("# Mobile roadmap\n", "Notebooks/Projects/x-1234.md")
        assertEquals("Mobile roadmap", noteTaskBody("Notebooks/Projects/x-1234.md", title).getString("title"))
    }

    @Test fun `a note restored to the top of the Vault names no folder`() {
        assertNull(VaultRules.restoredPlace("vault", "Notebooks/a.md"))
        assertEquals("Projects", VaultRules.restoredPlace("original", "Notebooks/Projects/a.md"))
        assertEquals("Agent-HQ", VaultRules.restoredPlace("folder", "Notebooks/Projects/Agent-HQ/a.md"))
    }

    @Test fun `an older HQ that does not say where reads as its own folder`() {
        assertEquals("Inbox", VaultRules.restoredPlace("original", "Notebooks/Inbox/a.md"))
    }

    @Test fun `opening Vault clears the dot and it stays clear until HQ files something new`() {
        assertTrue(VaultRules.dotCleared(visible = true, fresh = true, cleared = false))
        assertTrue(VaultRules.dotCleared(visible = false, fresh = false, cleared = true))
        assertFalse(VaultRules.dotCleared(visible = false, fresh = true, cleared = true))
        assertFalse(VaultRules.dotCleared(visible = false, fresh = false, cleared = false))
    }

    @Test fun `the dot comes back only for a marked note the phone had not heard of`() {
        val known = VaultRules.markedPaths(listOf(note("a.md", review = "new")))
        // The same marked note again: nothing fresh, so an opened Vault stays clear.
        assertFalse(VaultRules.hasFresh(known, VaultRules.markedPaths(listOf(note("a.md", review = "new")))))
        // A second marked note: fresh.
        assertTrue(VaultRules.hasFresh(known, VaultRules.markedPaths(listOf(note("a.md", review = "new"), note("b.md", review = "changed")))))
        // An unmarked note is never fresh.
        assertFalse(VaultRules.hasFresh(emptySet(), VaultRules.markedPaths(listOf(note("c.md", "hq")))))
    }

    @Test fun `the dot is shown from the server flag and the cleared state through the real reducer`() {
        val recent = listOf(note("b.md", "hq", "new"))
        // Elsewhere, something fresh arrives: the dot shows.
        val fresh = VaultRules.hasFresh(emptySet(), VaultRules.markedPaths(recent))
        val away = VaultReducer.signals(NotesUi(dotCleared = true), recent, true, VaultRules.dotCleared(visible = false, fresh = fresh, cleared = true))
        assertTrue(away.dot)
        // Vault is open when the same answer arrives: no dot.
        val open = VaultReducer.signals(NotesUi(), recent, true, VaultRules.dotCleared(visible = true, fresh = fresh, cleared = false))
        assertFalse(open.dot)
        // No server flag: no dot, whatever else.
        assertFalse(VaultReducer.signals(NotesUi(), emptyList(), false, false).dot)
    }
}
