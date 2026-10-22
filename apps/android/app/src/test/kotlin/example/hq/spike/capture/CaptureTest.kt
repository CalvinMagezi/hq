package example.hq.spike.capture

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CaptureTest {
    @Test fun aSharedLinkGetsAPreviewWithoutTheSchemeOrWww() {
        val p = CaptureWords.preview("Why Compose beat the WebView for chat", "https://www.example.com/compose-chat")!!
        assertEquals("Why Compose beat the WebView for chat", p.title)
        assertEquals("example.com/compose-chat", p.where)
    }

    @Test fun aLinkWithNoTitleUsesItsHost() {
        assertEquals("example.com", CaptureWords.preview(null, "see https://example.com/")!!.title)
        assertNull(CaptureWords.preview("Hello", "no link here"))
    }

    @Test fun theSubjectBecomesTheTitleElseTheFirstLineThatIsNotALink() {
        assertEquals("Subject", CaptureWords.titleFor("  Subject ", "body"))
        assertEquals("A thought", CaptureWords.titleFor(null, "https://example.com\nA thought\nmore"))
        assertEquals("", CaptureWords.titleFor(null, "https://example.com"))
    }

    @Test fun aNoteIsTitledByWhatWasTypedElseItsFirstLine() {
        assertEquals("Mine", CaptureWords.noteTitle(" Mine ", "x"))
        assertEquals("First line", CaptureWords.noteTitle("", "\n## First line\nsecond"))
        assertEquals("Shared note", CaptureWords.noteTitle("", ""))
    }

    @Test fun assigneesAreSplitCleanedAndUnique() {
        assertEquals(listOf("hq", "mobile"), CaptureWords.assignees("hq, @mobile  hq"))
        assertEquals(20, CaptureWords.assignees((1..30).joinToString(" ") { "a$it" }).size)
    }

    @Test fun aTaskCarriesItsRepeatKeyAndOnlyWhatWasSet() {
        val b = CaptureWords.taskBody("capture-1", " Check it ", "", "high", null, emptyList())
        assertEquals("Check it", b.getString("title"))
        assertEquals("capture-1", b.getString("external_id"))
        assertEquals("high", b.getString("priority"))
        assertEquals(false, b.has("description"))
        assertEquals(false, b.has("assignees"))
        assertEquals(listOf("hq"), (0 until 1).map { CaptureWords.taskBody("i", "t", "n", null, "2026-10-24", listOf("hq")).getJSONArray("assignees").getString(it) })
    }

    @Test fun aShareOpensOnTheOwnersChoiceAndLastMeansTheLastUsed() {
        assertEquals(CaptureMode.Task, CapturePrefs.startMode(null, "task", CaptureMode.Note))
        assertEquals(CaptureMode.Chat, CapturePrefs.startMode(null, CapturePrefs.LAST, CaptureMode.Chat))
        assertEquals(CaptureMode.Task, CapturePrefs.startMode("task", "note", CaptureMode.Note))
        assertEquals(CaptureMode.Note, CapturePrefs.startMode("nonsense", "nonsense", CaptureMode.Chat))
    }

    @Test fun theChatDraftHasEachPartOnce() {
        assertEquals("Title\n\nBody", CaptureWords.chatDraft("Title", "Body"))
        assertEquals("https://example.com", CaptureWords.chatDraft("https://example.com", "https://example.com"))
    }

    @Test fun theOutboxKeepsWhatWasSavedAcrossAReopen() {
        val file = File.createTempFile("outbox", ".json").also { it.delete() }
        val box = Outbox(file)
        box.add(Queued("a", Queued.KIND_NOTE, "N", CaptureWords.noteBody("Notebooks/Inbox", "N", "body"), 1L))
        box.add(Queued("b", Queued.KIND_TASK, "T", CaptureWords.taskBody("b", "T", "", null, null, emptyList()), 2L))
        box.fail("b", "HQ did not accept it")
        val again = Outbox(file).all()
        assertEquals(listOf("a", "b"), again.map { it.id })
        assertEquals("HQ did not accept it", again[1].failure)
        assertEquals("body", again[0].payload.getString("content"))
        Outbox(file).retry("b")
        assertNull(Outbox(file).all()[1].failure)
        Outbox(file).remove("a")
        assertEquals(listOf("b"), Outbox(file).all().map { it.id })
        file.delete()
    }

    @Test fun anOutboxThatIsMissingOrBrokenIsEmpty() {
        val file = File.createTempFile("outbox", ".json")
        file.writeText("not json")
        assertEquals(0, Outbox(file).all().size)
        file.delete()
        assertEquals(0, Outbox(file).all().size)
    }

    @Test fun aWidgetRowShowsTheLastPartOfTheDisplayId() {
        assertEquals("061", Widgets.numberOf("PERSONAL-INBOX-061"))
        assertEquals("T1", Widgets.numberOf("T1"))
    }
}
