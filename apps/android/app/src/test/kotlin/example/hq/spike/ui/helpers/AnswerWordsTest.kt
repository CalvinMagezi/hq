package example.hq.spike.ui.helpers

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AnswerWordsTest {
    @Test fun theHelpersNameComesFromItsQuestion() {
        assertEquals("Claude", AnswerWords.asker("Claude needs your OK"))
        assertEquals("Codex on the Mac mini", AnswerWords.asker("Codex on the Mac mini needs your OK"))
        assertNull(AnswerWords.asker("Delete 3 old backups?"))
        assertNull(AnswerWords.asker(" needs your OK"))
    }

    @Test fun allowingAHelperSaysItIsWorkingAgain() {
        assertEquals(AnswerWords.Say("Allowed. Claude is working again.", "Sent at 14:33."), AnswerWords.answered(true, true, "Claude", "14:33"))
    }

    @Test fun decliningAHelperSaysItIsWaitingAndWhatToDoNext() {
        assertEquals(AnswerWords.Say("Not now. Claude is waiting.", "Sent at 14:33. You can tell it what to do instead."), AnswerWords.answered(true, false, "Claude", "14:33"))
    }

    @Test fun aHelperWithoutAKnownNameIsStillNamedPlainly() {
        assertEquals("Allowed. The helper is working again.", AnswerWords.answered(true, true, null, "09:05").title)
    }

    @Test fun aTaskApprovalHasNoOneToName() {
        assertEquals(AnswerWords.Say("Allowed.", "Sent at 14:33."), AnswerWords.answered(false, true, null, "14:33"))
        assertEquals(AnswerWords.Say("Not now. Nothing was done.", "Sent at 14:33."), AnswerWords.answered(false, false, null, "14:33"))
    }

    @Test fun theSendingLineIsTheBoardsWordsWithOneEllipsisCharacter() {
        assertEquals("Sending your answer…", AnswerWords.sending.title)
        assertNull(AnswerWords.sending.body)
    }

    @Test fun answersTurnedOffSayWhereToAnswer() {
        assertEquals(AnswerWords.Say("Answers from notifications are off", "Open the app to answer."), AnswerWords.answersOff)
    }
}
