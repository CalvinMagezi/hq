package example.hq.spike.ui.notes

import example.hq.spike.data.HqClient.NoteItem
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Vault view model's state changes, run without its coroutines. */
class VaultReducerTest {
    private fun note(path: String, review: String? = null) = NoteItem(path, "", "", "", false, "hq", review)
    private val task = PendingUndo.TaskAdded("task-1", "Notebooks/a.md")
    private val deleted = PendingUndo.NoteDeleted("trash/a.md")

    @Test fun `make a task then delete then Undo takes back the note and leaves the task`() {
        val ui = VaultReducer.pend(VaultReducer.pend(NotesUi(), task), deleted)
        val (what, cleared) = VaultReducer.undone(ui)
        assertEquals(deleted, what)
        assertEquals(PendingUndo.None, cleared.pending)
    }

    @Test fun `delete then make a task then Undo takes back the task only`() {
        val ui = VaultReducer.pend(VaultReducer.pend(NotesUi(), deleted), task)
        assertEquals(task, VaultReducer.undone(ui).first)
    }

    @Test fun `a second Undo after the first does nothing`() {
        val (_, cleared) = VaultReducer.undone(VaultReducer.pend(NotesUi(), deleted))
        assertEquals(PendingUndo.None, VaultReducer.undone(cleared).first)
    }

    @Test fun `the snackbar timing out leaves nothing to undo`() {
        assertEquals(PendingUndo.None, VaultReducer.expired(VaultReducer.pend(NotesUi(), task)).pending)
    }

    @Test fun `opening a new note unmarks it at once`() {
        val ui = VaultReducer.seen(NotesUi(recent = listOf(note("a.md", "new"), note("b.md", "new"))), "a.md")
        assertNull(ui.recent.first { it.path == "a.md" }.review)
        assertEquals("new", ui.recent.first { it.path == "b.md" }.review)
    }

    @Test fun `a refresh asked for before the note was opened does not bring the mark back`() {
        val opened = VaultReducer.seen(NotesUi(recent = listOf(note("a.md", "new"))), "a.md")
        // The answer to the older request arrives now, still carrying the mark.
        val late = VaultReducer.signals(opened, listOf(note("a.md", "new")), hasNew = true, dotCleared = false)
        assertNull(late.recent.single().review)
        assertFalse("the flag was only for the note just opened", late.hasNew)
    }

    @Test fun `once HQ agrees the note is seen it is forgotten so a later change shows`() {
        val opened = VaultReducer.seen(NotesUi(recent = listOf(note("a.md", "new"))), "a.md")
        val agreed = VaultReducer.signals(opened, listOf(note("a.md")), hasNew = false, dotCleared = false)
        assertTrue(agreed.seenLocally.isEmpty())
        val changed = VaultReducer.signals(agreed, listOf(note("a.md", "changed")), hasNew = true, dotCleared = false)
        assertEquals("changed", changed.recent.single().review)
        assertTrue(changed.hasNew)
    }

    @Test fun `keeping a changed note clears its mark the same way`() {
        val kept = VaultReducer.seen(NotesUi(recent = listOf(note("a.md", "changed"))), "a.md")
        assertNull(kept.recent.single().review)
        assertNull(VaultReducer.signals(kept, listOf(note("a.md", "changed")), true, false).recent.single().review)
    }

    @Test fun `a flag for another unseen note stays`() {
        val opened = VaultReducer.seen(NotesUi(recent = listOf(note("a.md", "new"))), "a.md")
        assertTrue(VaultReducer.signals(opened, listOf(note("a.md", "new"), note("b.md", "new")), true, false).hasNew)
    }

    @Test fun `the same inputs always give the same state`() {
        val ui = VaultReducer.seen(NotesUi(recent = listOf(note("a.md", "new"))), "a.md")
        val answer = listOf(note("a.md", "new"))
        assertEquals(VaultReducer.signals(ui, answer, true, false), VaultReducer.signals(ui, answer, true, false))
    }

    @Test fun `a new note made into a task is named by its title`() {
        val title = VaultRules.titleOf("---\ntitle: Quarterly plan\n---\nbody", "Notebooks/Inbox/2026-11-12-0931.md")
        assertEquals("Quarterly plan", noteTaskBody("Notebooks/Inbox/2026-11-12-0931.md", title).getString("title"))
    }
}

class PhotoShotTest {
    @Test fun `a taken photo goes to the chat and nothing is thrown away`() {
        val shot = PhotoShot().apply { begin("photo-1.jpg") }
        val ended = shot.finish(true)
        assertEquals("photo-1.jpg", ended.handOver)
        assertNull(ended.discard)
    }

    @Test fun `a cancelled photo is deleted and not sent`() {
        val ended = PhotoShot("photo-1.jpg").finish(false)
        assertNull(ended.handOver)
        assertEquals("photo-1.jpg", ended.discard)
    }

    @Test fun `a second camera answer with nothing pending does nothing`() {
        val shot = PhotoShot("photo-1.jpg")
        shot.finish(true)
        val again = shot.finish(true)
        assertNull(again.handOver)
        assertNull(again.discard)
    }

    @Test fun `a name kept across a rotation still finishes`() {
        val saved = PhotoShot().apply { begin("photo-2.jpg") }.pending
        assertEquals("photo-2.jpg", PhotoShot(saved).finish(true).handOver)
    }

    @Test fun `only old captures are swept and never the one being taken`() {
        val now = 10 * CaptureFiles.KEEP_MS
        val files = listOf("old.jpg" to 0L, "current.jpg" to 0L, "fresh.jpg" to now - 1000)
        assertEquals(listOf("old.jpg"), CaptureFiles.stale(files, now, current = "current.jpg"))
    }
}
