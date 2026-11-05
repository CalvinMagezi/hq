package example.hq.spike.download

import example.hq.spike.measure.DataPurpose
import java.io.ByteArrayInputStream
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class DownloadRulesTest {
    private val big = 36_000_000L

    @Test fun smallJustStarts() = assertEquals(Decision.Start, DownloadRules.decide(10_000_000, NetKind.Mobile, true, false))

    @Test fun mobileAsksAndConsentHoldsExceptRoaming() {
        assertEquals(Decision.Ask(AskKind.Mobile), DownloadRules.decide(big, NetKind.Mobile, false, false))
        assertEquals(Decision.Start, DownloadRules.decide(big, NetKind.Mobile, false, true))
        assertEquals(Decision.Ask(AskKind.Roaming), DownloadRules.decide(big, NetKind.Roaming, false, true))
    }

    @Test fun wifiRules() {
        assertEquals(Decision.Start, DownloadRules.decide(big, NetKind.Free, false, false))
        assertEquals(Decision.Ask(AskKind.PaidWifi), DownloadRules.decide(big, NetKind.PaidWifi, false, false))
        assertEquals(Decision.Ask(AskKind.WifiQuestion), DownloadRules.decide(big, NetKind.UnknownWifi, false, false))
        assertEquals(Decision.Ask(AskKind.AnyNetwork), DownloadRules.decide(big, NetKind.Free, true, false))
    }

    @Test fun classify() {
        assertEquals(NetKind.None, DownloadRules.classify(false, false, false, false, false, null))
        assertEquals(NetKind.Roaming, DownloadRules.classify(true, true, false, true, true, null))
        assertEquals(NetKind.PaidWifi, DownloadRules.classify(true, false, true, true, false, null))
        assertEquals(NetKind.UnknownWifi, DownloadRules.classify(true, false, true, false, false, null))
        assertEquals(NetKind.Free, DownloadRules.classify(true, false, true, false, false, false))
    }

    @Test fun spaceAndWords() {
        assertEquals(36_000_000 + 40_000_000 + 10_000_000, DownloadRules.spaceNeeded(36_000_000, 40_000_000))
        assertEquals(2 * big + 10_000_000, DownloadRules.updateSpaceNeeded(big))
        assertNull(DownloadRules.spaceShort(100, 50))
        assertEquals(30L, DownloadRules.spaceShort(20, 50))
        assertEquals(57L, DownloadRules.sizeMb(57_267_600))
        assertEquals("3.4", DownloadRules.doneText(3_460_000))
        assertEquals("16", DownloadRules.doneText(16_900_000))
        assertEquals(20L, DownloadRules.leftMb(36_000_000, 16_000_001))
        assertEquals(99, DownloadRules.percent(36_000_000, 36_000_000))
        assertEquals(44, DownloadRules.percent(16_000_000, 36_000_000))
        assertEquals(1L, DownloadRules.usedMb(10))
        assertEquals(0L, DownloadRules.usedMb(0))
    }

    @Test fun expiryRetryStack() {
        assertFalse(DownloadRules.expired(0, 14 * DownloadRules.DAY_MS))
        assertTrue(DownloadRules.expired(0, 14 * DownloadRules.DAY_MS + 1))
        assertEquals(30_000L, DownloadRules.retryAfter(0))
        assertEquals(600_000L, DownloadRules.retryAfter(2))
        assertNull(DownloadRules.retryAfter(3))
        assertTrue(DownloadRules.stacks("a", "b", 1.5f))
        assertTrue(DownloadRules.stacks("Wait for Wi-Fi", "Download \u00b7 3 MB (long)", 1f))
        assertFalse(DownloadRules.stacks("Wait for Wi-Fi", "Download \u00b7 36 MB", 1f))
    }
}

class LargeDownloadTest {
    @get:Rule val tmp = TemporaryFolder()

    private val data = ByteArray(12_000_000) { (it % 251).toByte() }
    private val sha = MessageDigest.getInstance("SHA-256").digest(data).joinToString("") { "%02x".format(it) }

    private class Env(val data: ByteArray) : DownloadEnv {
        var net = NetKind.Free
        var any = false
        var free = Long.MAX_VALUE
        var ignoreRange = false
        var failures = 0
        var clock = 1_000_000L
        val counted = CopyOnWriteArrayList<Pair<Boolean, Long>>()
        val timers = ArrayList<Pair<Long, () -> Unit>>()
        var cutAfter = -1
        override fun now() = clock
        override fun net() = net
        override fun askAnyNetwork() = any
        override fun freeBytes() = free
        override fun sleep(ms: Long) { clock += ms }
        override fun undoMs() = 10_000L
        override fun open(url: String, from: Long, ifRange: String?): Opened {
            if (failures > 0) { failures--; return Opened(503, 0, null, null) }
            val start = if (ignoreRange) 0 else from.toInt()
            var slice = data.copyOfRange(start, data.size)
            if (cutAfter >= 0) { slice = slice.copyOf(cutAfter); cutAfter = -1; failures = 10 }
            return Opened(if (start > 0) 206 else 200, start.toLong(), "e", ByteArrayInputStream(slice))
        }
        override fun count(purpose: DataPurpose, paid: Boolean, bytes: Long) { counted.add(paid to bytes) }
        override fun rememberWifi(paid: Boolean) { net = if (paid) NetKind.PaidWifi else NetKind.Free }
        override fun later(delayMs: Long, run: () -> Unit) { timers.add(delayMs to run) }
        override fun spawn(run: () -> Unit) = run()
    }

    private fun item(sha: String = this.sha) = Item("a", "AgentHQ 1", "https://example.com/a.apk", data.size.toLong(), sha, DataPurpose.AppUpdates, data.size.toLong(), "a.apk")
    private fun make(env: Env, sha: String = this.sha) = LargeDownload(item(sha), tmp.root, env)

    @Test fun freeWifiDownloadsAndVerifies() {
        val env = Env(data)
        val d = make(env)
        d.request()
        assertEquals(DlState.Done(0), d.state.value)
        assertTrue(d.file.readBytes().contentEquals(data))
    }

    @Test fun mobileAsksThenCountsPaidBytes() {
        val env = Env(data).also { it.net = NetKind.Mobile }
        val d = make(env)
        d.request()
        assertEquals(DlState.Asking(AskKind.Mobile, 0), d.state.value)
        assertEquals(0L, d.kept())
        d.accept()
        assertEquals(DlState.Done(data.size.toLong()), d.state.value)
        assertTrue(env.counted.all { it.first })
    }

    @Test fun notEnoughSpaceShowsHowMuchToFree() {
        val env = Env(data).also { it.free = 5_000_000 }
        val d = make(env)
        d.request()
        val s = d.state.value as DlState.NoSpace
        assertEquals(2 * data.size + 10_000_000L, s.need)
        assertEquals(s.need - 5_000_000, s.short)
    }

    @Test fun tryAgainAfterAMismatchAsksAgainFromNothing() {
        val d = make(Env(data), "b".repeat(64))
        d.request()
        assertEquals(DlState.Mismatch, d.state.value)
        d.tryAgain()
        assertEquals(DlState.Mismatch, d.state.value)
        assertEquals(0L, d.kept())
    }

    @Test fun hashMismatchDeletesEverything() {
        val d = make(Env(data), "b".repeat(64))
        d.request()
        assertEquals(DlState.Mismatch, d.state.value)
        assertFalse(d.file.exists())
        assertEquals(0L, d.kept())
    }

    @Test fun resumesWithRangeAndKeepsBytes() {
        val env = Env(data).also { it.cutAfter = 5_000_000 }
        val d = make(env)
        d.request()
        assertEquals(DlState.ServerFailing(5_000_000), d.state.value)
        assertEquals(5_000_000L, d.kept())
        env.failures = 0
        d.request()
        assertEquals(DlState.Done(0), d.state.value)
    }

    @Test fun a200OnAPaidNetworkAsksToStartOver() {
        val env = Env(data).also { it.cutAfter = 5_000_000 }
        val d = make(env)
        d.request()
        env.failures = 0
        env.net = NetKind.Mobile
        env.ignoreRange = true
        d.resumeWithConsent()
        assertEquals(DlState.Asking(AskKind.StartOver, 5_000_000), d.state.value)
    }

    @Test fun a200OnFreeWifiStartsOverQuietly() {
        val env = Env(data).also { it.cutAfter = 5_000_000 }
        val d = make(env)
        d.request()
        env.failures = 0
        env.ignoreRange = true
        d.request()
        assertEquals(DlState.Done(0), d.state.value)
    }

    @Test fun serverFailureRetriesThreeTimesThenStops() {
        val env = Env(data).also { it.failures = 10 }
        val d = make(env)
        val start = env.clock
        d.request()
        assertEquals(DlState.ServerFailing(0), d.state.value)
        assertEquals(30_000L + 120_000 + 600_000, env.clock - start)
    }

    @Test fun cancelUndoAndExpiry() {
        val env = Env(data).also { it.cutAfter = 3_000_000 }
        val d = make(env)
        d.request()
        d.cancel()
        assertEquals(DlState.Cancelled(3_000_000), d.state.value)
        env.failures = 0
        d.undo()
        assertEquals(DlState.Done(0), d.state.value)
        val e2 = Env(data).also { it.cutAfter = 3_000_000 }
        val d2 = LargeDownload(item().copy(id = "b", dest = "b.apk"), tmp.root, e2)
        d2.request()
        d2.cancel()
        e2.timers.single().second()
        assertEquals(0L, d2.kept())
    }

    @Test fun leavingWifiWaitsForConsentAndWifiReturnsToStart() {
        val env = Env(data)
        val d = make(env)
        d.request()
        val e2 = Env(data).also { it.net = NetKind.Mobile }
        val w = LargeDownload(item().copy(id = "c", dest = "c.apk"), tmp.root, e2)
        w.request()
        w.waitForWifi()
        assertEquals(DlState.WaitingWifi, w.state.value)
        e2.net = NetKind.Free
        w.networkChanged()
        assertEquals(DlState.Done(0), w.state.value)
    }

    @Test fun partialPastFourteenDaysIsRemovedOnRestore() {
        val env = Env(data).also { it.cutAfter = 2_000_000 }
        val d = make(env)
        d.request()
        File(tmp.root, "a.apk.part").setLastModified(0)
        val again = make(env.also { it.clock = 15 * DownloadRules.DAY_MS })
        again.restore()
        assertEquals(0L, again.kept())
    }

    @Test fun wifiAnswerIsAsked() {
        val env = Env(data).also { it.net = NetKind.UnknownWifi }
        val d = make(env)
        d.request()
        assertEquals(DlState.Asking(AskKind.WifiQuestion, 0), d.state.value)
        d.answerWifi(false)
        assertEquals(DlState.Done(0), d.state.value)
    }
}
