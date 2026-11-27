package example.hq.spike.ui.chat.blocks

import java.io.File
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * The phone reads `[[wikilinks]]` the way the server and the web app do. All three are checked against
 * one file, crates/hq-vault/tests/fixtures/links_cases.json, so a link that opens a note in one place
 * cannot quietly stop opening it in another.
 *
 * Only the wikilinks written in plain text are compared. Code spans, escapes and properties are the
 * server's and the web renderer's to skip, and the phone's text renderer does not read them.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class LinksCorpusTest {
    // Unit tests run from the app module, three folders below the repository root.
    private val corpus = JSONObject(File("../../../crates/hq-vault/tests/fixtures/links_cases.json").readText())
    private val wikilink = Regex("!?\\[\\[([^\\[\\]\\n]+)]]")

    @Test fun everyPlainWikilinkOpensTheNoteTheCorpusSays() {
        val cases = corpus.getJSONArray("cases")
        var checked = 0
        for (i in 0 until cases.length()) {
            val case = cases.getJSONObject(i)
            val text = case.getString("text")
            if (text.contains('`') || text.contains('\\') || text.startsWith("---")) continue
            val want = (0 until case.getJSONArray("links").length()).map { case.getJSONArray("links").getJSONObject(it) }
                .filter { it.optString("syntax") != "markdown" && it.optString("target").isNotEmpty() }
            val got = wikilink.findAll(text).map { NoteRefs.wikilink(it.groupValues[1]) }.filter { it.first.isNotEmpty() }.toList()
            assertEquals("${case.getString("name")}: how many wikilinks", want.size, got.size)
            want.zip(got).forEach { (truth, link) ->
                assertEquals("${case.getString("name")}: the note it opens", truth.getString("target"), link.first)
                if (truth.has("alias")) assertEquals("${case.getString("name")}: its label", truth.getString("alias"), link.second)
                checked++
            }
        }
        assertTrue("the corpus gave something to check ($checked)", checked >= 15)
    }
}
