package example.hq.spike.capture

import java.io.File
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class OutboxSafetyTest {
    private fun item(id: String, failure: String? = null) = Queued(id, Queued.KIND_NOTE, id, JSONObject().put("folder", "Notebooks/Inbox").put("title", id).put("content", ""), 1L, failure)

    @Test fun aFileThatCannotBeReadIsKeptAsItIsNotWrittenOver() {
        val file = File.createTempFile("outbox", ".json")
        file.writeText("{ half")
        val box = Outbox(file)
        box.add(item("new"))
        assertEquals(listOf("new"), box.all().map { it.id })
        assertEquals("{ half", File(file.path + ".bad").readText())
        File(file.path + ".bad").delete()
        file.delete()
    }

    @Test fun overTheLimitTheOldestRefusedGoFirstAndNothingWaitingIsDropped() {
        val file = File.createTempFile("outbox", ".json").also { it.delete() }
        val box = Outbox(file)
        box.add(item("refused", "HQ did not accept it"))
        repeat(Outbox.MAX_ITEMS) { box.add(item("w$it")) }
        val ids = box.all().map { it.id }
        assertEquals(Outbox.MAX_ITEMS, ids.size)
        assertTrue("refused" !in ids)
        assertTrue("w0" in ids)
        box.add(item("one-more"))
        assertEquals(Outbox.MAX_ITEMS + 1, box.all().size)
        file.delete()
    }

    @Test fun aSharedTextThatHoldsItsTitleOrTheOtherWayRoundIsSaidOnce() {
        assertEquals("A long thought about it", CaptureWords.chatDraft("A long thought", "A long thought about it"))
        assertEquals("A long thought about it", CaptureWords.chatDraft("A long thought about it", "A long thought"))
        assertEquals("Title\n\nBody", CaptureWords.chatDraft("Title", "Body"))
        assertEquals("Only body", CaptureWords.chatDraft("", " Only body "))
    }
}
