package example.hq.spike.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ReplyFilesTest {
    private fun reply(files: String) = JSONObject(
        """{"message_id":"a1","thread_id":"t","role":"assistant","content":"done","created_at":"2026-11-05T00:00:00Z","meta":{"files":$files}}""",
    )

    @Test
    fun aRepliesFilesAreRead() {
        val m = StoredMessage.from(reply("""[{"path":"Exports/2026-11/AI stocks.xlsx","name":"AI stocks.xlsx","mime":"application/vnd.ms-excel","size":2048,"kind":"sheet"}]"""))
        assertEquals(listOf(ReplyFile("AI stocks.xlsx", "Exports/2026-11/AI stocks.xlsx", "application/vnd.ms-excel", 2048)), m.files)
    }

    @Test
    fun aPathThatLeavesTheVaultIsDropped() {
        val m = StoredMessage.from(reply("""[{"path":"../x.xlsx","name":"x"},{"path":"/etc/passwd","name":"p"},{"path":"","name":"e"}]"""))
        assertTrue(m.files.isEmpty())
    }

    @Test
    fun aReplyWithoutFilesHasNone() {
        val m = StoredMessage.from(JSONObject("""{"message_id":"a","role":"assistant","content":"hi"}"""))
        assertTrue(m.files.isEmpty())
    }
}
