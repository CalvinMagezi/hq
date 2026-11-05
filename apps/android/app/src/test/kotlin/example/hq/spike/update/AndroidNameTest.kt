package example.hq.spike.update

import org.junit.Assert.assertEquals
import org.junit.Test

class AndroidNameTest {
    @Test fun anApiLevelIsNamedByItsAndroidRelease() {
        assertEquals("Android 10", UpdateLogic.androidName(29))
        assertEquals("Android 12L", UpdateLogic.androidName(32))
        assertEquals("Android 8.0", UpdateLogic.androidName(26))
        assertEquals("Android 8.1", UpdateLogic.androidName(27))
        assertEquals("a newer Android", UpdateLogic.androidName(37))
        assertEquals("10", UpdateLogic.androidNumber(29))
        assertEquals(null, UpdateLogic.androidNumber(37))
        assertEquals("a newer Android", UpdateLogic.androidName(99))
    }
}
