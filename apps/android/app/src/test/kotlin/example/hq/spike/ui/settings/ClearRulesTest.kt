package example.hq.spike.ui.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ClearRulesTest {
    @Test fun oneClearRunsAtATimeAndEachEndingIsNamed() {
        assertEquals(ClearState.Busy, ClearRules.begin(ClearState.Idle))
        assertEquals(ClearState.Busy, ClearRules.begin(ClearState.Failed))
        assertNull(ClearRules.begin(ClearState.Busy))
        assertEquals(ClearState.Done, ClearRules.end(true))
        assertEquals(ClearState.Failed, ClearRules.end(false))
    }
}
