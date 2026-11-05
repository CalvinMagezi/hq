package example.hq.spike.ui.helpers

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import example.hq.spike.testing.testWords
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class AnswerWordsTest {
    private val w get() = testWords()

    @Test fun theHelpersNameComesFromItsQuestion() {
        assertEquals("Claude", AnswerWords.asker("Claude needs your OK"))
        assertEquals("Codex on the Mac mini", AnswerWords.asker("Codex on the Mac mini needs your OK"))
        assertNull(AnswerWords.asker("Delete 3 old backups?"))
        assertNull(AnswerWords.asker(" needs your OK"))
    }

    @Test fun allowingAHelperSaysItIsWorkingAgain() {
        assertEquals(AnswerWords.Say("Allowed. Claude is working again.", "Sent at 14:33."), AnswerWords.answered(w, true, true, "Claude", "14:33"))
    }

    @Test fun decliningAHelperSaysItIsWaitingAndWhatToDoNext() {
        assertEquals(AnswerWords.Say("Not now. Claude is waiting.", "Sent at 14:33. You can tell it what to do instead."), AnswerWords.answered(w, true, false, "Claude", "14:33"))
    }

    @Test fun aHelperWithoutAKnownNameIsStillNamedPlainly() {
        assertEquals("Allowed. The helper is working again.", AnswerWords.answered(w, true, true, null, "09:05").title)
    }

    @Test fun aTaskApprovalHasNoOneToName() {
        assertEquals(AnswerWords.Say("Allowed.", "Sent at 14:33."), AnswerWords.answered(w, false, true, null, "14:33"))
        assertEquals(AnswerWords.Say("Not now. Nothing was done.", "Sent at 14:33."), AnswerWords.answered(w, false, false, null, "14:33"))
    }

    @Test fun theSendingLineIsTheBoardsWordsWithOneEllipsisCharacter() {
        assertEquals("Sending your answer…", AnswerWords.sending(w).title)
        assertNull(AnswerWords.sending(w).body)
    }

    @Test fun anItemIsAnsweredElsewhereOnlyWhenTheHistoryPositivelySaysSo() {
        val history = listOf("a" to "approved", "b" to "dismissed", "c" to "pending")
        assertEquals(true, AnswerWords.answeredElsewhere("a", history))
        assertEquals(true, AnswerWords.answeredElsewhere("b", history))
        assertEquals(false, AnswerWords.answeredElsewhere("c", history))
        assertEquals(false, AnswerWords.answeredElsewhere("missing", history))
        assertEquals(false, AnswerWords.answeredElsewhere("a", emptyList()))
    }

    @Test fun theAlreadyAnsweredCardUsesTheBoardsWords() {
        assertEquals(AnswerWords.Say("Already answered", "Someone answered this somewhere else."), AnswerWords.alreadyAnswered(w))
    }
}
