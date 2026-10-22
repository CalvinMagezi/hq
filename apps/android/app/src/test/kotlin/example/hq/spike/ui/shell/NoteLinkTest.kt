package example.hq.spike.ui.shell

import org.junit.Assert.assertEquals
import org.junit.Test

class NoteLinkTest {
    @Test fun aNoteWithSpacesAndSignsInItsNameOpensFromItsOwnLink() {
        for (path in listOf("Notebooks/Inbox/My Note.md", "Notebooks/Inbox/Why #1 & more?.md", "Notebooks/Projects/Plan 100% (v2).md")) {
            assertEquals(path, Links.parse(Links.noteLink(path))?.path)
        }
    }

    @Test fun aPlainPathIsLeftReadable() {
        assertEquals("hq://notes/Notebooks/Inbox/A.md", Links.noteLink("Notebooks/Inbox/A.md"))
    }
}
