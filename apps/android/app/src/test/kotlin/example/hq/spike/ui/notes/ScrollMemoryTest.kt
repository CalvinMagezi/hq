package example.hq.spike.ui.notes

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ScrollMemoryTest {
    @Test fun aNoteIsRestoredToWhereItWasLeft() {
        val m = ScrollMemory()
        m.save("Notebooks/a.md", 14, 120)
        assertEquals(14 to 120, m.restore("Notebooks/a.md"))
        assertNull(m.restore("Notebooks/b.md"))
    }

    @Test fun theLatestPlaceWinsAndAFreshOpenStartsAtTheTop() {
        val m = ScrollMemory()
        m.save("a.md", 3, 0)
        m.save("a.md", 9, 40)
        assertEquals(9 to 40, m.restore("a.md"))
        m.forget("a.md")
        assertNull(m.restore("a.md"))
    }
}
