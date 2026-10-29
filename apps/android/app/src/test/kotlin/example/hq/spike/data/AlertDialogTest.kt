package example.hq.spike.data

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

/** The dialog a helper's notice carries, as the server's notifications list sends it in `metadata.dialog`. */
class AlertDialogTest {
    private fun meta(dialog: String) = JSONObject("""{"extra":{"source_task":"session-supervisor"},"dialog":$dialog}""")

    @Test fun aReadDialogComesWithItsYesOnceLabel() {
        val d = alertDialog(
            meta(
                """{"session_id":"hs-1","seq":7,"kind":"path","question":"Do you want to allow this?","command":"/tmp",
                "options":[{"id":1,"label":"Yes"},{"id":2,"label":"Yes, and remember this path","remember":true}],
                "approve":{"id":1,"label":"Yes"},"answered":false}""",
            ),
        )!!
        assertEquals("hs-1", d.sessionId)
        assertEquals(7L, d.seq)
        assertEquals("path", d.kind)
        assertEquals("Do you want to allow this?", d.question)
        assertEquals("/tmp", d.command)
        assertEquals("Yes", d.approveLabel)
        assertFalse(d.answered)
    }

    @Test fun noYesOnceAnswerMeansNoAllowLabel() {
        val d = alertDialog(meta("""{"session_id":"hs-1","seq":3,"kind":"question","question":"Which colour?","command":null,"options":[],"approve":null,"answered":false}"""))!!
        assertNull(d.approveLabel)
        assertNull(d.command)
    }

    @Test fun aNoticeWithoutADialogOrWithAHalfOneHasNone() {
        assertNull(alertDialog(null))
        assertNull(alertDialog(JSONObject("""{"extra":{}}""")))
        assertNull(alertDialog(meta("""{"seq":3}""")))
        assertNull(alertDialog(meta("""{"session_id":"hs-1"}""")))
    }
}
