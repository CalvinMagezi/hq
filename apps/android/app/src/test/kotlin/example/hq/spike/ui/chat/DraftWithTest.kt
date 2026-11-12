package example.hq.spike.ui.chat

import org.junit.Assert.assertEquals
import org.junit.Test

class DraftWithTest {
    @Test fun `an import line goes after what is typed and never over it`() {
        assertEquals("Please summarise this\n\nAdd this to my Vault as a note", draftWith("Please summarise this ", "Add this to my Vault as a note"))
    }

    @Test fun `an empty field gets the line alone`() {
        assertEquals("Add this", draftWith("", "Add this"))
        assertEquals("Add this", draftWith("  \n", "Add this"))
    }
}
