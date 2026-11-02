package example.hq.spike.voice

import org.junit.Assert.assertEquals
import org.junit.Test

class InvocationTest {
    @Test
    fun theMisheardGreetingIsDropped() {
        // What the A32 heard when the owner said "Hey HQ, what time is it".
        assertEquals("What time is it", Invocation.strip("Ue what time is it"))
        assertEquals("What time is it", Invocation.strip("Hey ue what time is it"))
        assertEquals("Add milk", Invocation.strip("hey h q, add milk"))
        assertEquals("What's due", Invocation.strip("Hey aitch queue what's due"))
        assertEquals("Call mum", Invocation.strip("Okay HQ call mum"))
        // Kroko writes punctuation and joins the words.
        assertEquals("What time is it?", Invocation.strip("Ahq, what time is it?"))
        assertEquals("What time is it?", Invocation.strip("Hey, HQ. What time is it?"))
        assertEquals("What time is it?", Invocation.strip("HeyHQ what time is it?"))
    }

    @Test
    fun ordinaryOpeningsAreKept() {
        assertEquals("What time is it", Invocation.strip("What time is it"))
        assertEquals("Hey there", Invocation.strip("Hey there"))
        assertEquals("You have mail", Invocation.strip("You have mail"))
        assertEquals("Uem is a word", Invocation.strip("Uem is a word"))
        assertEquals("Hugo called", Invocation.strip("Hugo called"))
        assertEquals("Ahead of time", Invocation.strip("Ahead of time"))
        assertEquals("Achoo", Invocation.strip("Achoo"))
    }

    @Test
    fun aGreetingAloneIsNotEmptied() {
        assertEquals("Hey HQ", Invocation.strip("Hey HQ"))
    }
}
