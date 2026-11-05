package example.hq.spike.ui.settings

import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class BackupFactsTest {
    private val now = Instant.parse("2026-10-22T12:00:00Z")
    private fun hoursAgo(h: Long) = now.minusSeconds(h * 3600).toEpochMilli()
    private val on = BackupConfig(true, 24, 7, null)
    private val off = hoursAgo(10)

    @Test fun onlyTheBackupBlockOfTheConfigIsRead() {
        val yaml = "llm:\n  api_key: secret\nbackup:\n  enabled: true\n  dir: /data/x/backups\n  key: abc\n  every_hours: 12   # twice a day\n  keep: 3\n\nweb:\n  enabled: false\n"
        assertEquals(BackupConfig(true, 12, 3, "/data/x/backups"), BackupWords.parseConfig(yaml))
        assertNull(BackupWords.parseConfig("llm:\n  api_key: secret\n"))
        assertEquals(24, BackupWords.parseConfig("backup:\n  enabled: true\n")!!.everyHours)
        assertEquals(false, BackupWords.parseConfig("backup:\n  enabled: false\n")!!.enabled)
    }

    @Test fun theStatusFileIsReadOrIgnored() {
        assertEquals(BackupStatus(true, "wrote it", 3155, 0), BackupWords.parseStatus("""{"ok":true,"message":"wrote it","files":3155,"left_out":0}"""))
        assertNull(BackupWords.parseStatus("not json"))
    }

    @Test fun noBackupConfiguredOffersToSetOneUp() {
        val c = BackupWords.card(BackupFacts(copyOffPhoneMs = off), now)
        assertEquals(BackupWords.Tone.Good, c.tone)
        assertEquals("Backup is not set up", c.title)
        assertEquals(true, c.offerSetUp)
        assertEquals("Backup is not set up", BackupWords.card(BackupFacts(config = BackupConfig(false, 24, 7, null), copyOffPhoneMs = off), now).title)
    }

    @Test fun aWorkingBackupSaysWhenAndHowOften() {
        val c = BackupWords.card(BackupFacts(on, BackupStatus(true, "", 3155, 0, "/x/hq-backup-A.hqbak"), hoursAgo(5), "hq-backup-A.hqbak", copyOffPhoneMs = off), now)
        assertEquals(BackupWords.Tone.Good, c.tone)
        assertEquals("Backup is on", c.title)
        assertEquals("Last backup 5 h ago, 3155 files. A new one is made every 24 hours and the newest 7 are kept.", c.body)
        assertEquals(false, c.offerSetUp)
        assertEquals("Last backup 1 d ago, 10 files. A new one is made every 24 hours and the newest 7 are kept. 3 files were left out.",
            BackupWords.card(BackupFacts(on, BackupStatus(true, "", 10, 3, "/x/hq-backup-A.hqbak"), hoursAgo(30), "hq-backup-A.hqbak", copyOffPhoneMs = off), now).body)
        assertEquals("Last backup 5 h ago. A new one is made every 24 hours and the newest 7 are kept.",
            BackupWords.card(BackupFacts(on, BackupStatus(true, "", 3155, 0, "/x/hq-backup-OLD.hqbak"), hoursAgo(5), "hq-backup-B.hqbak", copyOffPhoneMs = off), now).body)
    }

    @Test fun aFailedOrOldOrMissingBackupIsSaidPlainly() {
        assertEquals("The last backup did not finish", BackupWords.card(BackupFacts(on, BackupStatus(false, "disk full", 0, 0), hoursAgo(1)), now).title)
        assertEquals("disk full", BackupWords.card(BackupFacts(on, BackupStatus(false, "disk full", 0, 0), hoursAgo(1)), now).body)
        assertEquals("The last backup is old", BackupWords.card(BackupFacts(on, null, hoursAgo(60), copyOffPhoneMs = off), now).title)
        assertEquals("No backup has finished yet. A new one is made every 24 hours and the newest 7 are kept.", BackupWords.card(BackupFacts(on), now).body)
    }

    @Test fun theStatesAreChosenInOrder() {
        assertEquals(BackupState.Server, BackupWords.state(BackupFacts(on, readable = false, server = true), now))
        assertEquals(BackupState.CouldNotCheck, BackupWords.state(BackupFacts(on, readable = false), now))
        assertEquals(BackupState.Off, BackupWords.state(BackupFacts(), now))
        assertEquals(BackupState.Failed, BackupWords.state(BackupFacts(on, BackupStatus(false, "x", 0, 0), hoursAgo(1)), now))
        assertEquals(BackupState.Waiting, BackupWords.state(BackupFacts(on, firstSeenOnMs = hoursAgo(47)), now))
        assertEquals(BackupState.Overdue, BackupWords.state(BackupFacts(on, firstSeenOnMs = hoursAgo(48)), now))
    }

    @Test fun overdueIsMeasuredExactly() {
        assertEquals(BackupState.Recent, BackupWords.state(BackupFacts(on, newestMs = hoursAgo(48) + 60_000), now))
        assertEquals(BackupState.Overdue, BackupWords.state(BackupFacts(on, newestMs = hoursAgo(48)), now))
    }

    @Test fun theRowAndTheCardWarnTogether() {
        val onlyHere = BackupFacts(on, newestMs = hoursAgo(1))
        assertEquals(true, BackupWords.warns(onlyHere, now))
        assertEquals("Your notes are only on this phone", BackupWords.homeLine(onlyHere, now))
        assertEquals(BackupWords.Tone.Warn, BackupWords.card(onlyHere, now).tone)
        val copied = onlyHere.copy(copyOffPhoneMs = hoursAgo(24 * 8))
        assertEquals(true, BackupWords.warns(copied, now))
        val fresh = onlyHere.copy(copyOffPhoneMs = hoursAgo(24 * 6))
        assertEquals(false, BackupWords.warns(fresh, now))
        assertEquals(false, BackupWords.warns(BackupFacts(server = true), now))
        assertEquals(BackupWords.Tone.Good, BackupWords.card(BackupFacts(server = true), now).tone)
    }
}
