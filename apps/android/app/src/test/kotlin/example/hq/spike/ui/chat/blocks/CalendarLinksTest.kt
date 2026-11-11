package example.hq.spike.ui.chat.blocks

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CalendarLinksTest {
    @Test fun readsTheOneShape() {
        assertEquals(CalendarLink(42, 1000, 2000), CalendarLinks.parse("hq://calendar/event/42?begin=1000&end=2000"))
    }

    @Test fun refusesAnythingElse() {
        listOf(
            "hq://calendar/event/42?begin=1000&end=2000&x=1",
            "hq://calendar/event/42?begin=1000&end=2000#frag",
            "hq://calendar/event/42?end=2000&begin=1000",
            "hq://calendar/event/42?begin=1000",
            "hq://calendar/event/42",
            "hq://calendar/event/-1?begin=1&end=2",
            "hq://calendar/event/1?begin=-1&end=2",
            "hq://calendar/event/1?begin=1&end=-2",
            "hq://calendar/event/99999999999999999999?begin=1&end=2",
            "hq://calendar/event/1?begin=99999999999999999999&end=2",
            "hq://calendar/event/1?begin=1e3&end=2",
            "hq://calendar/event/abc?begin=1&end=2",
            "hq://calendar/event/1/2?begin=1&end=2",
            "hq://other/event/1?begin=1&end=2",
            "hq://calendar@evil.example/event/1?begin=1&end=2",
            "https://calendar/event/1?begin=1&end=2",
            "javascript:alert(1)",
            "content://com.android.calendar/events/1",
            " hq://calendar/event/1?begin=1&end=2",
            "",
        ).forEach { assertNull(it, CalendarLinks.parse(it)) }
    }
}
