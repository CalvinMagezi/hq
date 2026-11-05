package example.hq.spike.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class SavedCopyTest {
    @get:Rule val tmp = TemporaryFolder()

    @Test fun whatIsWrittenIsReadBack() {
        val c = SavedCopy(tmp.newFolder())
        c.write("tasks", SavedCopy.Source.Phone, JSONObject().put("a", 1), 5L)
        val r = c.read("tasks")!!
        assertEquals(5L, r.atMs)
        assertEquals(1, r.body.getInt("a"))
        assertNull(c.read("none"))
    }

    @Test fun signingOutOfAServerClearsOnlyItsCopies() {
        val c = SavedCopy(tmp.newFolder())
        c.write("a", SavedCopy.Source.Phone, JSONObject(), 1L)
        c.write("b", SavedCopy.Source.Server, JSONObject(), 1L)
        c.clearServer()
        assertTrue(c.read("a") != null)
        assertNull(c.read("b"))
        c.clearAll()
        assertNull(c.read("a"))
    }

    @Test fun aBigListIsHalvedToFitAPlace() {
        val c = SavedCopy(tmp.newFolder())
        val list = org.json.JSONArray().also { a -> repeat(2000) { a.put("x".repeat(500)) } }
        c.write("big", SavedCopy.Source.Phone, JSONObject().put("items", list), 1L, listKey = "items")
        assertTrue(c.totalBytes() <= 256 * 1024 + 512)
        assertTrue(c.read("big")!!.body.getJSONArray("items").length() in 1..1999)
    }

    @Test fun noTempFileIsLeftBehind() {
        val dir = tmp.newFolder()
        SavedCopy(dir).write("p", SavedCopy.Source.Phone, JSONObject(), 1L)
        assertEquals(listOf("p.json"), dir.list()!!.toList())
    }
}
