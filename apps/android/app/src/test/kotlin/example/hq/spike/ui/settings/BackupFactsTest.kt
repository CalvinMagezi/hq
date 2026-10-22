package example.hq.spike.ui.settings

import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class BackupFactsTest {
    private val now = Instant.parse("2026-10-22T12:00:00Z")
    private fun hoursAgo(h: Long) = now.minusSeconds(h * 3600).toEpochMilli()
    private val on = BackupConfig(true, 24, 7, null)

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
        val c = BackupWords.card(BackupFacts(), now)
        assertEquals(BackupWords.Tone.Warn, c.tone)
        assertEquals("Backup is not set up", c.title)
        assertEquals(true, c.offerSetUp)
        assertEquals("Backup is not set up", BackupWords.card(BackupFacts(config = BackupConfig(false, 24, 7, null)), now).title)
    }

    @Test fun aWorkingBackupSaysWhenAndHowOften() {
        val c = BackupWords.card(BackupFacts(on, BackupStatus(true, "", 3155, 0), hoursAgo(5)), now)
        assertEquals(BackupWords.Tone.Good, c.tone)
        assertEquals("Backup is on", c.title)
        assertEquals("Last backup 5 h ago, 3155 files. A new one is made every 24 hours and the newest 7 are kept.", c.body)
        assertEquals(false, c.offerSetUp)
        assertEquals("Last backup 1 d ago, 10 files. A new one is made every 24 hours and the newest 7 are kept. 3 files were left out.",
            BackupWords.card(BackupFacts(on, BackupStatus(true, "", 10, 3), hoursAgo(30)), now).body)
    }

    @Test fun aFailedOrOldOrMissingBackupIsSaidPlainly() {
        assertEquals("The last backup did not finish", BackupWords.card(BackupFacts(on, BackupStatus(false, "disk full", 0, 0), hoursAgo(1)), now).title)
        assertEquals("disk full", BackupWords.card(BackupFacts(on, BackupStatus(false, "disk full", 0, 0), hoursAgo(1)), now).body)
        assertEquals("The last backup is old", BackupWords.card(BackupFacts(on, null, hoursAgo(60)), now).title)
        assertEquals("No backup has finished yet. A new one is made every 24 hours and the newest 7 are kept.", BackupWords.card(BackupFacts(on), now).body)
    }
}
