package example.hq.spike.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class NoteWireTest {
    private fun item(json: String) = NoteWire.item(JSONObject(json))

    @Test fun `an older HQ that sends no marks reads as the owner's note`() {
        val n = item("""{"path":"Notebooks/a.md"}""")
        assertEquals(WRITTEN_BY_YOU, n.writtenBy)
        assertNull(n.review)
        assertNull(n.why)
        assertEquals(emptyList<String>(), n.tags)
    }

    @Test fun `null fields are absent, not the word null`() {
        val n = item("""{"path":"a.md","written_by":null,"review":null,"why":null,"tags":null}""")
        assertEquals(WRITTEN_BY_YOU, n.writtenBy)
        assertNull(n.review)
        assertNull(n.why)
        assertEquals(emptyList<String>(), n.tags)
    }

    @Test fun `known marks are read`() {
        val n = item("""{"path":"a.md","written_by":"hq","review":"changed","why":"Saved from your chat, 10 Nov","tags":["roadmap","android"]}""")
        assertEquals(WRITTEN_BY_HQ, n.writtenBy)
        assertEquals("changed", n.review)
        assertEquals("Saved from your chat, 10 Nov", n.why)
        assertEquals(listOf("roadmap", "android"), n.tags)
    }

    @Test fun `odd values fall back and odd tags are dropped`() {
        val n = item("""{"path":"a.md","written_by":42,"review":"accepted","why":"  ","tags":["a",null,7,"",{"x":1}," b "]}""")
        assertEquals(WRITTEN_BY_YOU, n.writtenBy)
        assertNull(n.review)
        assertNull(n.why)
        assertEquals(listOf("a", "b"), n.tags)
    }

    @Test fun `an unknown writer reads as the owner`() {
        assertEquals(WRITTEN_BY_YOU, item("""{"path":"a.md","written_by":"robot"}""").writtenBy)
        assertEquals("import", item("""{"path":"a.md","written_by":"import"}""").writtenBy)
    }

    @Test fun `a note body carries the same marks`() {
        val b = NoteWire.body(JSONObject("""{"content":"x","written_by":"hq","review":"new","tags":["t"]}"""), "Notebooks/a.md")
        assertEquals("Notebooks/a.md", b.path)
        assertEquals(WRITTEN_BY_HQ, b.writtenBy)
        assertEquals("new", b.review)
        assertEquals(listOf("t"), b.tags)
        assertNull(NoteWire.body(JSONObject("""{"content":"x","review":null}"""), "a.md").review)
    }

    @Test fun `restored_to defaults to the original folder`() {
        assertEquals(RESTORED_ORIGINAL, NoteWire.restoredTo(JSONObject("""{"path":"a.md"}""")))
        assertEquals(RESTORED_ORIGINAL, NoteWire.restoredTo(JSONObject("""{"restored_to":null}""")))
        assertEquals(RESTORED_ORIGINAL, NoteWire.restoredTo(JSONObject("""{"restored_to":"moon"}""")))
        assertEquals(RESTORED_FOLDER, NoteWire.restoredTo(JSONObject("""{"restored_to":"folder"}""")))
        assertEquals(RESTORED_VAULT, NoteWire.restoredTo(JSONObject("""{"restored_to":"vault"}""")))
    }

    @Test fun `an unreadable answer is not the same as no answer`() {
        assertEquals(true, example.hq.spike.ui.notes.VaultRules.isOffline(0))
        assertEquals(false, example.hq.spike.ui.notes.VaultRules.isOffline(HqUnavailable.UNREADABLE))
        assertEquals(false, example.hq.spike.ui.notes.VaultRules.isOffline(500))
    }
}
