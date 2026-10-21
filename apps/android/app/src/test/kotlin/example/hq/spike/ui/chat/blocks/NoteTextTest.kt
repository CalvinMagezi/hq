package example.hq.spike.ui.chat.blocks

import org.junit.Assert.assertEquals
import org.junit.Test

class NoteTextTest {
    @Test fun frontmatterIsDropped() {
        assertEquals("## Title\n\nBody", NoteText.withoutFrontmatter("---\ntitle: x\ntags: [a]\n---\n## Title\n\nBody"))
    }

    @Test fun aNoteWithoutFrontmatterIsUntouched() {
        assertEquals("## Title\n---\nmore", NoteText.withoutFrontmatter("## Title\n---\nmore"))
    }

    @Test fun anUnclosedBlockIsLeftAlone() {
        assertEquals("---\ntitle: x\nBody", NoteText.withoutFrontmatter("---\ntitle: x\nBody"))
    }
}
