package example.hq.spike.update

import org.junit.Assert.assertEquals
import org.junit.Test

class AndroidNameTest {
    @Test fun anApiLevelIsNamedByItsAndroidRelease() {
        assertEquals("Android 10", UpdateLogic.androidName(29))
        assertEquals("Android 12", UpdateLogic.androidName(32))
        assertEquals("a newer Android", UpdateLogic.androidName(99))
    }
}
