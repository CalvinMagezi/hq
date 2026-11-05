package example.hq.spike.update

import android.content.pm.PackageInstaller
import example.hq.spike.download.AskKind
import example.hq.spike.download.Btn
import example.hq.spike.download.DlState
import example.hq.spike.download.DownloadWords
import example.hq.spike.download.Item
import example.hq.spike.download.Op
import example.hq.spike.measure.DataPurpose
import example.hq.spike.testing.testWords
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

class UpdateNoticesDueTest {
    private val day = 24L * 3600 * 1000

    @Test fun firstSightPostsOnceThenAWeekLater() {
        assertEquals(UpdateNotices.Kind.Found, UpdateNotices.due(0, 0, 100, false))
        assertNull(UpdateNotices.due(100, 100, 100 + 6 * day, false))
        assertEquals(UpdateNotices.Kind.Reminder, UpdateNotices.due(100, 100, 100 + 7 * day, false))
    }

    @Test fun anUrgentVersionRemindsEachDay() {
        assertNull(UpdateNotices.due(100, 100, 100 + day - 1, true))
        assertEquals(UpdateNotices.Kind.Reminder, UpdateNotices.due(100, 100, 100 + day, true))
    }

    @Test fun aClockThatWentBackPostsNothing() = assertNull(UpdateNotices.due(100, 500, 200, false))
}

class ProblemKindTest {
    @Test fun androidsAnswersBecomeProblems() {
        assertNull(UpdateLogic.problemOf(PackageInstaller.STATUS_SUCCESS, null))
        assertEquals(ProblemKind.Stopped, UpdateLogic.problemOf(PackageInstaller.STATUS_FAILURE_ABORTED, null))
        assertEquals(ProblemKind.Blocked, UpdateLogic.problemOf(PackageInstaller.STATUS_FAILURE_BLOCKED, null))
        assertEquals(ProblemKind.AutoBlocker, UpdateLogic.problemOf(PackageInstaller.STATUS_FAILURE_BLOCKED, "Blocked by Auto Blocker"))
        assertEquals(ProblemKind.NotVerified, UpdateLogic.problemOf(PackageInstaller.STATUS_FAILURE, "developer verification failed"))
        assertEquals(ProblemKind.NotInstalled, UpdateLogic.problemOf(PackageInstaller.STATUS_FAILURE, null))
    }

    @Test fun androidNumbers() {
        assertEquals("8.0", UpdateLogic.androidNumber(26))
        assertEquals("12L", UpdateLogic.androidNumber(32))
        assertEquals("16", UpdateLogic.androidNumber(36))
        assertNull(UpdateLogic.androidNumber(99))
    }
}

@RunWith(RobolectricTestRunner::class)
class DownloadRowWordsTest {
    private val w = testWords()
    private val big = Item("update", "AgentHQ 1.5.0", "https://example.com/a.apk", 36_000_000, "a".repeat(64), DataPurpose.AppUpdates, 36_000_000, "a.apk")
    private val small = big.copy(size = 5_000_000)

    private fun ops(m: example.hq.spike.download.RowModel) = m.buttons.map(Btn::op)

    @Test fun readyOffersDownload() = assertTrue(Op.Download in ops(DownloadWords.row(w, big, DlState.Ready)))

    @Test fun runningShowsABarAndPauseCancel() {
        val m = DownloadWords.row(w, big, DlState.Downloading(16_000_000, false))
        assertNotNull(m.progress)
        assertEquals(listOf(Op.Pause, Op.Cancel), ops(m))
    }

    @Test fun smallOnesOnlySayDownloading() {
        val m = DownloadWords.row(w, small, DlState.Downloading(1_000_000, false))
        assertTrue(m.spinner)
        assertNull(m.progress)
        assertEquals("Downloading\u2026", m.status)
    }

    @Test fun pausedOffersResume() = assertTrue(Op.Resume in ops(DownloadWords.row(w, big, DlState.PausedOwner(16_000_000))))

    @Test fun aMismatchOffersTryAgainAndNoSizeSaysNothingKept() {
        val m = DownloadWords.row(w, big, DlState.Mismatch)
        assertTrue(Op.TryAgain in ops(m))
        assertTrue(m.warn)
    }

    @Test fun everyAskHasATitleAndBody() {
        AskKind.entries.forEach { k ->
            val (t, b) = DownloadWords.ask(w, big, k, 1_000_000)
            assertFalse(t.isBlank() || b.isBlank())
        }
    }
}
