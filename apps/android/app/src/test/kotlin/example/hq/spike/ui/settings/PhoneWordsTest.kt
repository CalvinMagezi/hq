package example.hq.spike.ui.settings

import example.hq.spike.testing.testWords

import android.app.ApplicationExitInfo
import java.time.Instant
import example.hq.spike.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class PhoneWordsTest {
    @org.junit.Before fun english() = java.util.Locale.setDefault(java.util.Locale.ENGLISH)

    private val now = Instant.parse("2026-10-22T12:00:00Z")

    @Test fun everythingFineOnASamsungNamesTheOneThingItCannotCheck() {
        val page = PhoneWords.keepRunning(testWords(), KeepFacts(autoRevokeOff = true, samsung = true), now)
        assertEquals("One thing Android may still stop", page.noticeTitle)
        assertEquals("Three settings are fine. A fourth, in Samsung's own battery menu, cannot be checked from here.", page.notice)
        assertTrue(page.warn)
        assertTrue(page.sleepingApps)
        assertEquals(listOf("Unrestricted", "Off for AgentHQ", "Allowed"), page.checked.map { it.value })
    }

    @Test fun twoCheckedSettingsReadAsTwoWhenThereIsNoAutoRevoke() {
        val page = PhoneWords.keepRunning(testWords(), KeepFacts(autoRevokeOff = null, samsung = true), now)
        assertEquals("Two settings are fine. A third, in Samsung's own battery menu, cannot be checked from here.", page.notice)
    }

    @Test fun aSettingThatIsNotFineOffersItsAndroidPage() {
        val page = PhoneWords.keepRunning(testWords(), KeepFacts(batteryUnrestricted = false, autoRevokeOff = false, notificationsAllowed = false), now)
        assertEquals("3 things to fix", page.noticeTitle)
        assertEquals(listOf(PhoneFix.Battery, PhoneFix.AutoRevoke, PhoneFix.Notifications), page.checked.map { it.fix })
        assertTrue(page.warn)
    }

    @Test fun backgroundRestrictedIsWorseThanOptimised() {
        assertEquals("Restricted", PhoneWords.keepRunning(testWords(), KeepFacts(backgroundRestricted = true), now).checked[0].value)
    }

    @Test fun everythingFineOffASamsungIsPlainlyFine() {
        val page = PhoneWords.keepRunning(testWords(), KeepFacts(autoRevokeOff = true), now)
        assertFalse(page.warn)
        assertFalse(page.sleepingApps)
        assertEquals("Good", PhoneWords.keepSummary(testWords(), KeepFacts(autoRevokeOff = true), now))
    }

    @Test fun theHomeRowCountsWhatIsToFix() {
        assertEquals("2 to fix", PhoneWords.keepSummary(testWords(), KeepFacts(batteryUnrestricted = false, notificationsAllowed = false), now))
        assertEquals("Good · 1 thing to check", PhoneWords.keepSummary(testWords(), KeepFacts(samsung = true), now))
    }

    @Test fun theHistoryRowSaysWhenAndWhy() {
        val row = PhoneWords.keepRunning(testWords(), KeepFacts(lastStop = LastStop(ApplicationExitInfo.REASON_LOW_MEMORY, now.minusSeconds(2 * 86_400L))), now).history
        assertEquals(FactRow("Last stopped by Android", "2 d ago · low memory"), row)
        assertEquals("Nothing recorded", PhoneWords.keepRunning(testWords(), KeepFacts(), now).history.value)
    }

    @Test fun onlyAnExitAndroidChoseCounts() {
        assertTrue(PhoneWords.androidStopped(ApplicationExitInfo.REASON_LOW_MEMORY))
        assertTrue(PhoneWords.androidStopped(ApplicationExitInfo.REASON_SIGNALED))
        assertFalse(PhoneWords.androidStopped(ApplicationExitInfo.REASON_USER_REQUESTED))
        assertFalse(PhoneWords.androidStopped(ApplicationExitInfo.REASON_EXIT_SELF))
        assertFalse(PhoneWords.androidStopped(ApplicationExitInfo.REASON_CRASH))
    }

    @Test fun channelsReadTheBoardsLines() {
        assertEquals(R.string.channel_needs_line, PhoneWords.channelLineRes(ChannelFacts("needs", "Needs you", 4, true)))
        assertEquals(R.string.channel_updates_line, PhoneWords.channelLineRes(ChannelFacts("updates", "Updates", 3, false)))
        assertEquals(R.string.channel_app_updates_line, PhoneWords.channelLineRes(ChannelFacts("app_updates", "App updates", 2, false)))
        assertEquals(R.string.channel_downloads_line, PhoneWords.channelLineRes(ChannelFacts("downloads", "Downloads", 2, false)))
        assertEquals(R.string.channel_hub_line, PhoneWords.channelLineRes(ChannelFacts("hub_min", "HQ is running", 1, false)))
    }

    @Test fun everyChannelReadsOffWhileTheAppSwitchIsOff() {
        assertEquals(R.string.channel_off_line, PhoneWords.channelLineRes(ChannelFacts("updates", "Updates", 3, false), appOff = true))
        assertEquals(R.string.channel_updates_line, PhoneWords.channelLineRes(ChannelFacts("updates", "Updates", 3, false), appOff = false))
    }

    @Test fun aChannelTurnedOffSaysSo() {
        assertEquals(R.string.channel_off_line, PhoneWords.channelLineRes(ChannelFacts("needs", "Needs you", 0, false)))
    }

    @Test fun anUnknownExitReasonIsStillWorded() {
        assertEquals("killed by the system", PhoneWords.exitWords(ApplicationExitInfo.REASON_SIGNALED))
        assertEquals("frozen", PhoneWords.exitWords(14))
        assertNull(null)
    }

    @Test fun theCaptureRowSaysWhereASharegoesAndWhatWaits() {
        assertEquals("Sharing opens a new note", PhoneWords.captureSummary(CaptureFacts()))
        assertEquals("Sharing opens a new task", PhoneWords.captureSummary(CaptureFacts("task")))
        assertEquals("Sharing opens the last one you used", PhoneWords.captureSummary(CaptureFacts("last")))
        val q = example.hq.spike.capture.Queued("a", "note", "N", org.json.JSONObject(), 1L)
        assertEquals("Sharing opens a new note \u00b7 1 waiting to send", PhoneWords.captureSummary(CaptureFacts(waiting = listOf(q))))
    }

    @Test fun aQueuedItemSaysWhatItIsAndWhyItIsStillThere() {
        val q = example.hq.spike.capture.Queued("a", "task", "T", org.json.JSONObject(), 1L)
        assertEquals("Task \u00b7 Waiting for HQ", PhoneWords.queuedLine(q))
        assertEquals("Task \u00b7 HQ did not accept it", PhoneWords.queuedLine(q.copy(failure = "HQ did not accept it")))
        assertEquals("Note \u00b7 Waiting for HQ", PhoneWords.queuedLine(q.copy(kind = "note")))
    }
}
