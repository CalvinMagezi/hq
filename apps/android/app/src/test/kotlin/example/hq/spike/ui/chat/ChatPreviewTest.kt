package example.hq.spike.ui.chat

import org.junit.Assert.assertEquals
import org.junit.Test

class ChatPreviewTest {
    @Test fun aPreviewIsOnePlainLine() {
        assertEquals("Pinned the note as version 1: done. Next, the roadmap.", ChatPreview.snippet("Pinned the note as `version 1`: **done**.\n\nNext, the roadmap."))
        assertEquals("Deeper investigation: private networking", ChatPreview.snippet("# Deeper investigation: private networking"))
    }

    @Test fun aPreviewCutInsideAMarkLosesTheOpenMark() {
        assertEquals("Yes, that\u2019s a stronger direction: HQ on t", ChatPreview.snippet("Yes, that\u2019s a stronger direction: **HQ on t"))
        assertEquals("Open the file src/main", ChatPreview.snippet("Open the file `src/main"))
    }

    @Test fun nothingAtAllIsNothing() {
        assertEquals("", ChatPreview.snippet(null))
        assertEquals("", ChatPreview.snippet("  "))
    }
}
