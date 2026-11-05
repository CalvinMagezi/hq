package example.hq.spike.data

import org.junit.Assert.assertEquals
import org.junit.Test

class ChatEventPartTwoTest {
    @Test
    fun theNewFramesParse() {
        assertEquals(ChatEvent.ReplySoFar("t", true, "Hi", "m"), ChatEvent.parse("""{"type":"reply_so_far","thread_id":"t","running":true,"content":"Hi","message_id":"m"}"""))
        assertEquals(ChatEvent.TurnEnd("t", null, false, true), ChatEvent.parse("""{"type":"turn_end","thread_id":"t","waiting_for_owner":true}"""))
    }
}
