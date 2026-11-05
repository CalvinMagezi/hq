package example.hq.spike.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Updates wait for HQ to be free, and a bad version can be pulled (wave-0/02-safe-mode.md, sections 6.15 to 6.21). */
class SafeUpdateLogicTest {
    private val sha = "a".repeat(64)
    private fun m(code: Long = 10, rollout: Int = 100, withdrawn: List<Long> = emptyList(), minSdk: Int = 26, seq: Long = 1) =
        ChannelManifest("1.$code", code, UpdateLogic.APK_URL_PREFIX + "x/a.apk", sha, minSdk, seq = seq, rollout = rollout, withdrawn = withdrawn)

    private fun json(extra: String) = """{"version":"1.0","versionCode":5,"url":"${UpdateLogic.APK_URL_PREFIX}x/a.apk","sha256":"$sha","minSdk":26$extra}"""

    @Test fun oldManifestStillParsesWithDefaults() {
        val p = UpdateLogic.parse(json(""))!!
        assertEquals(100, p.rollout); assertFalse(p.urgent); assertTrue(p.withdrawn.isEmpty()); assertEquals(0, p.seq)
        assertTrue(p.switches.taskOutbox); assertFalse(p.switches.installWithoutConfirm); assertFalse(p.switches.lockScreenAssistant)
    }

    @Test fun newFieldsParse() {
        val p = UpdateLogic.parse(json(""","seq":9,"rollout":20,"urgent":true,"withdrawn":[3,4],"size":123,"notes":{"en":["a","b"],"fr":["c"]},"switches":{"saveCopy":false,"lockScreenAssistant":true,"lockScreenAssistOffModels":["SM-A325F"]}"""))!!
        assertEquals(9, p.seq); assertEquals(20, p.rollout); assertTrue(p.urgent); assertEquals(listOf(3L, 4L), p.withdrawn)
        assertEquals(listOf("c"), UpdateLogic.notesFor(p, "fr")); assertEquals(listOf("a", "b"), UpdateLogic.notesFor(p, "de"))
        assertFalse(p.switches.saveCopy)
        assertFalse(p.switches.lockScreenAssistAllowed("sm-a325f")); assertTrue(p.switches.lockScreenAssistAllowed("Pixel"))
    }

    @Test fun notesAreCappedAtThreeLinesOfOneHundredTwentyCharacters() {
        assertNull(UpdateLogic.parse(json(""","notes":{"en":["1","2","3","4"]}""")))
        assertNull(UpdateLogic.parse(json(""","notes":{"en":["${"x".repeat(121)}"]}""")))
        assertNotNull(UpdateLogic.parse(json(""","notes":{"en":["${"x".repeat(120)}","2","3"]}""")))
        assertEquals(listOf("hi"), UpdateLogic.parse(json(""","notes":"hi""""))!!.notes["en"])
    }

    @Test fun badRolloutIsRefused() {
        assertNull(UpdateLogic.parse(json(""","rollout":101""")))
        assertNull(UpdateLogic.parse(json(""","rollout":-1""")))
    }

    @Test fun rolloutHoldsBackPhonesAboveTheBucketOnStableButNotOnMain() {
        assertTrue(UpdateLogic.verdict(m(rollout = 20), 5, 35, bucket = 19, ignoreRollout = false) is UpdateVerdict.Available)
        assertEquals(UpdateVerdict.UpToDate, UpdateLogic.verdict(m(rollout = 20), 5, 35, bucket = 20, ignoreRollout = false))
        assertTrue(UpdateLogic.verdict(m(rollout = 20), 5, 35, bucket = 99, ignoreRollout = true) is UpdateVerdict.Available)
    }

    @Test fun installedWithdrawnOffersTheFixOnlyWhenItIsGood() {
        val ok = UpdateLogic.verdict(m(code = 11, withdrawn = listOf(10)), 10, 35) as UpdateVerdict.InstalledWithdrawn
        assertNotNull(ok.fix)
        assertNull((UpdateLogic.verdict(m(code = 10, withdrawn = listOf(10)), 10, 35) as UpdateVerdict.InstalledWithdrawn).fix)
        assertNull((UpdateLogic.verdict(m(code = 11, withdrawn = listOf(10, 11)), 10, 35) as UpdateVerdict.InstalledWithdrawn).fix)
        assertNotNull((UpdateLogic.verdict(m(code = 11, withdrawn = listOf(10), rollout = 0), 10, 35, bucket = 50, ignoreRollout = false) as UpdateVerdict.InstalledWithdrawn).fix)
    }

    @Test fun aPulledCandidateIsOnlyNewsIfItWasOffered() {
        assertEquals(UpdateVerdict.Pulled("1.11"), UpdateLogic.verdict(m(code = 11, withdrawn = listOf(11)), 10, 35, wasOffered = true))
        assertEquals(UpdateVerdict.UpToDate, UpdateLogic.verdict(m(code = 11, withdrawn = listOf(11)), 10, 35, wasOffered = false))
    }

    @Test fun anOlderSeqIsARefusedReplay() {
        assertFalse(UpdateLogic.seqOk(m(seq = 4), 5))
        assertTrue(UpdateLogic.seqOk(m(seq = 5), 5))
    }

    @Test fun busyAnswerParses() {
        val b = UpdateLogic.parseBusy("""{"busy":true,"chat_turns":2,"backup":false}""")!!
        assertEquals(2, b.chatTurns); assertTrue(b.busy)
        assertNull(UpdateLogic.parseBusy("nope"))
    }

    @Test fun busyReasonOrderIsChatBackupOutboxDownload() {
        val all = BusyStatus(true, 1, true)
        assertEquals(BusyReason.Chat, UpdateLogic.busyReason(all, true, true))
        assertEquals(BusyReason.Backup, UpdateLogic.busyReason(BusyStatus(true, 0, true), true, true))
        assertEquals(BusyReason.Outbox, UpdateLogic.busyReason(BusyStatus(false, 0, false), true, true))
        assertEquals(BusyReason.Download, UpdateLogic.busyReason(BusyStatus(false, 0, false), false, true))
        assertNull(UpdateLogic.busyReason(BusyStatus(false, 0, false), false, false))
    }

    @Test fun hqNotAnsweringCountsAsFree() {
        assertNull(UpdateLogic.busyReason(null, true, false))
        assertEquals(BusyReason.Download, UpdateLogic.busyReason(null, true, true))
    }
}
