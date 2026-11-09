package example.hq.spike.ui.notes

import org.junit.Assert.assertEquals
import org.junit.Test

class RestoredFolderTest {
    @Test fun subfoldersHaveNotebooksPrefixStripped() {
        assertEquals("Work/Projects", restoredFolder("Notebooks/Work/Projects/note.md"))
        assertEquals("Inbox", restoredFolder("Notebooks/Inbox/todo.md"))
    }

    @Test fun topLevelNoteFallsBackToNotebooks() {
        assertEquals("Notebooks", restoredFolder("Notebooks/root.md"))
    }
}
