package example.hq.spike.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatEventTest {
    @Test
    fun textAndToolFramesParse() {
        assertEquals(ChatEvent.Text("t1", "hi"), ChatEvent.parse("""{"type":"text_delta","thread_id":"t1","content":"hi","thinking":false}"""))
        val tool = ChatEvent.parse("""{"type":"tool_start","thread_id":"t1","tool_name":"vault_search","tool_call_id":"c1","args":"{\"q\":\"x\"}"}""")
        assertEquals(ChatEvent.ToolStarted("t1", ToolStep("c1", "vault_search", "{\"q\":\"x\"}", null, null)), tool)
        assertEquals(ChatEvent.ToolFinished("t1", "c1", "3 notes"), ChatEvent.parse("""{"type":"tool_end","thread_id":"t1","tool_call_id":"c1","tool_name":"vault_search","result":"3 notes"}"""))
    }

    @Test
    fun turnStartCarriesTheSavedQuestionAndTheClientId() {
        val e = ChatEvent.parse(
            """{"type":"turn_start","thread_id":"t1","client_id":"local-1","user_message":{"message_id":"m1","thread_id":"t1","role":"user","content":"hello","created_at":"2026-10-21T07:00:00Z"}}""",
        ) as ChatEvent.TurnStart
        assertEquals("local-1", e.clientId)
        assertEquals("m1", e.userMessage?.id)
    }

    @Test
    fun endAndFailureAndRejection() {
        assertEquals(ChatEvent.TurnEnd("t1", "m2", false), ChatEvent.parse("""{"type":"turn_end","thread_id":"t1","message_id":"m2"}"""))
        assertEquals(ChatEvent.TurnEnd("t1", null, true), ChatEvent.parse("""{"type":"turn_end","thread_id":"t1","stopped":true,"message_id":null}"""))
        assertEquals(ChatEvent.Failed("t1", "boom"), ChatEvent.parse("""{"type":"error","thread_id":"t1","content":"boom"}"""))
        val r = ChatEvent.parse("""{"type":"chat_rejected","thread_id":"t1","client_id":"c","reason":"busy","running":true}""") as ChatEvent.Rejected
        assertTrue(r.running)
    }

    @Test
    fun unusedAndBrokenFramesGiveNull() {
        assertNull(ChatEvent.parse("""{"type":"step_credits","thread_id":"t1","turn":1,"delta":0.1}"""))
        assertNull(ChatEvent.parse("""{"type":"text_delta","thread_id":"t1","content":"x","thinking":true}"""))
        assertNull(ChatEvent.parse("not json"))
    }

    @Test
    fun savedMessagesKeepTheirToolSteps() {
        val m = StoredMessage.from(
            org.json.JSONObject(
                """{"message_id":"m3","thread_id":"t1","role":"assistant","content":"done","created_at":"x","meta":{"tool_steps":[{"id":"c1","name":"task_list","args":"{}","result":"2 tasks","duration_ms":120}],"stopped":true}}""",
            ),
        )
        assertEquals(listOf(ToolStep("c1", "task_list", "{}", "2 tasks", 120L)), m.tools)
        assertTrue(m.stopped)
    }
}
