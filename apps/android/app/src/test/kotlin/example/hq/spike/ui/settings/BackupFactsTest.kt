package example.hq.spike.ui.settings

import example.hq.spike.data.HqClient
import example.hq.spike.testing.testWords
import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** The words come from the generated catalogue, read through Robolectric the way a screen reads them. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class BackupFactsTest {
    private val now = Instant.parse("2026-10-22T12:00:00Z")
    private val w = testWords()
    private fun hoursAgo(h: Long) = now.minusSeconds(h * 3600).toEpochMilli()
    private val on = BackupConfig(true, 24, 7, "/d", "/k")
    private val off = hoursAgo(10)
    private fun card(f: BackupFacts) = BackupWords.card(w, f, now)
    private fun home(f: BackupFacts) = BackupWords.homeLine(w, f, now)

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

    @Test fun theStatesAreChosenInOrder() {
        assertEquals(BackupState.Server, BackupWords.state(BackupFacts(on, readable = false, server = true), now))
        assertEquals(BackupState.CouldNotCheck, BackupWords.state(BackupFacts(on, readable = false), now))
        assertEquals(BackupState.CouldNotCheck, BackupWords.state(BackupFacts(on, dirOutside = true), now))
        assertEquals(BackupState.Off, BackupWords.state(BackupFacts(), now))
        assertEquals(BackupState.Failed, BackupWords.state(BackupFacts(on, BackupStatus(false, "x", 0, 0), hoursAgo(1)), now))
        assertEquals(BackupState.Waiting, BackupWords.state(BackupFacts(on, firstSeenOnMs = hoursAgo(47)), now))
        assertEquals(BackupState.Overdue, BackupWords.state(BackupFacts(on, firstSeenOnMs = hoursAgo(48)), now))
    }

    @Test fun overdueIsMeasuredExactlyAtTwiceTheInterval() {
        assertEquals(BackupState.Recent, BackupWords.state(BackupFacts(on, newestMs = hoursAgo(48) + 60_000), now))
        assertEquals(BackupState.Overdue, BackupWords.state(BackupFacts(on, newestMs = hoursAgo(48)), now))
    }

    @Test fun notSetUpWithACopyOffThePhoneIsGrey() {
        val f = BackupFacts(copyOffPhoneMs = off)
        assertEquals("Not set up \u00b7 copy saved 10 h ago", home(f))
        val c = card(f)
        assertEquals(BackupWords.Tone.Good, c.tone)
        assertEquals("Backup is not set up", c.title)
        assertEquals(listOf(BackupWords.Action.SaveCopy), c.actions)
        assertFalse(BackupWords.warns(f, now))
    }

    @Test fun notSetUpAndOnlyOnThePhoneWarnsAndOffersACopy() {
        val f = BackupFacts()
        assertEquals("Not set up \u00b7 only on this phone", home(f))
        val c = card(f)
        assertEquals(BackupWords.Tone.Warn, c.tone)
        assertEquals("Backups are only on this phone", c.title)
        assertTrue(c.actions.contains(BackupWords.Action.SaveCopy))
        assertEquals("The copy is not locked with a password. Anyone with the file can read your Vault. Your keys are not in it.", c.lock)
    }

    @Test fun withSaveCopySwitchedOffTheCardIsGreyHasNoCopyButtonAndGotItIsShownOnce() {
        val f = BackupFacts(saveCopyOn = false)
        assertFalse(BackupWords.warns(f, now))
        assertEquals(BackupWords.Tone.Good, card(f).tone)
        assertEquals(listOf(BackupWords.Action.GotIt), card(f).actions)
        assertEquals(2, card(f).sentences.size)
        val done = card(f.copy(gotItDismissed = true))
        assertEquals(1, done.sentences.size)
        assertTrue(done.actions.isEmpty())
        assertNull(BackupWords.slot(w, f, now))
    }

    @Test fun aWorkingBackupSaysWhenAndHowOften() {
        val f = BackupFacts(on, BackupStatus(true, "", 3155, 0, "/x/hq-backup-A.hqbak"), hoursAgo(5), "hq-backup-A.hqbak", copyOffPhoneMs = off)
        val c = card(f)
        assertEquals(BackupWords.Tone.Good, c.tone)
        assertEquals("Backup is on", c.title)
        assertEquals("Last backup 5 h ago, 3155 files. A new one is made every 24 hours. The newest 7 are kept.", c.body)
        assertEquals("Copy saved off this phone 10 h ago.", c.small)
        assertEquals("Last backup 5 h ago", home(f))
        val left = card(f.copy(status = BackupStatus(true, "", 10, 3, "/x/hq-backup-A.hqbak"), newestMs = hoursAgo(30)))
        assertEquals("Last backup 1 d ago, 10 files. A new one is made every 24 hours. The newest 7 are kept. 3 files were left out.", left.body)
        val other = card(f.copy(status = BackupStatus(true, "", 3155, 0, "/x/hq-backup-OLD.hqbak")))
        assertEquals("Last backup 5 h ago. A new one is made every 24 hours. The newest 7 are kept.", other.body)
        assertEquals("Last backup 1 d ago, 1 file. A new one is made every 24 hours. The newest 7 are kept.", card(f.copy(status = BackupStatus(true, "", 1, 0, "/x/hq-backup-A.hqbak"), newestMs = hoursAgo(25))).body)
    }

    @Test fun aGoodBackupThatIsOnlyOnThePhoneWarnsAndAsksForACopy() {
        val f = BackupFacts(on, newestMs = hoursAgo(1))
        assertTrue(BackupWords.warns(f, now))
        assertEquals("Last backup 1 h ago \u00b7 phone only", home(f))
        val c = card(f)
        assertEquals(BackupWords.Tone.Warn, c.tone)
        assertEquals("If you lose it, you lose them. Last backup 1 h ago. Save a copy to a computer or a cloud drive.", c.body)
        assertTrue(BackupWords.warns(f.copy(copyOffPhoneMs = hoursAgo(24 * 8)), now))
        assertFalse(BackupWords.warns(f.copy(copyOffPhoneMs = hoursAgo(24 * 6)), now))
    }

    @Test fun aFailedBackupSaysWhatHqSaidAndHowOldTheNewestGoodOneIs() {
        val f = BackupFacts(on, BackupStatus(false, "disk full", 0, 0), hoursAgo(30))
        val c = card(f)
        assertEquals("The last backup did not finish", c.title)
        assertEquals("Your newest changes are not in a backup. The newest good one is from 1 d ago.", c.body)
        assertEquals("HQ said: disk full", c.said)
        assertEquals("HQ did not say why.", card(f.copy(status = BackupStatus(false, "", 0, 0))).said)
        assertEquals("Last backup did not finish", home(f))
        assertTrue(c.actions.containsAll(listOf(BackupWords.Action.SaveCopy, BackupWords.Action.CopyDetails)))
    }

    @Test fun waitingIsGreyAndOverdueSaysWhy() {
        val waiting = BackupFacts(on, firstSeenOnMs = hoursAgo(5))
        assertEquals("On \u00b7 first one not made yet", home(waiting))
        assertFalse(BackupWords.warns(waiting, now))
        val old = BackupFacts(on, newestMs = hoursAgo(60))
        assertEquals("Overdue \u00b7 last backup 2 d ago", home(old))
        assertEquals("The last backup is old", card(old).title)
        assertEquals("Last backup 2 d ago. A new one should be made every 24 hours. Something is stopping it. Restart HQ, or copy details for whoever set up HQ.", card(old).body)
        val paused = card(old.copy(paused = true))
        assertEquals("Backups run only while Background work is on. It is paused.", paused.sentences[1])
        assertTrue(paused.actions.contains(BackupWords.Action.OpenBackground))
        assertFalse(card(old).actions.contains(BackupWords.Action.OpenBackground))
    }

    @Test fun noBackupMadeNamesAMissingFolderOrKeyFile() {
        val f = BackupFacts(on.copy(keyFile = null), firstSeenOnMs = hoursAgo(50))
        assertEquals("Overdue \u00b7 none made yet", home(f))
        val c = card(f)
        assertEquals("No backup has been made", c.title)
        assertEquals("Backups are on, but none was made in 2 days. The backup settings have no folder or no key file. Copy details for whoever set up HQ.", c.body)
        assertEquals("Backups are on, but none was made in 2 days. Copy details for whoever set up HQ.", card(BackupFacts(on, firstSeenOnMs = hoursAgo(50))).body)
        assertEquals("Backups are on, but none was made in 36 hours.", card(BackupFacts(on.copy(everyHours = 18), firstSeenOnMs = hoursAgo(40))).sentences[0])
    }

    @Test fun couldNotCheckSaysWhichOfTheTwoReasons() {
        assertEquals("Could not check", home(BackupFacts(on, readable = false)))
        assertEquals("Backups go to a folder this app cannot open. It cannot say if they work.", card(BackupFacts(on, dirOutside = true)).body)
        assertEquals("Backup could not be checked", card(BackupFacts(on, readable = false)).title)
    }

    @Test fun aServerNeverWarnsAndNeverOffersACopy() {
        val f = BackupFacts(server = true, serverName = "hq.example.com")
        assertEquals("Backups run on your server HQ", home(f))
        assertFalse(BackupWords.warns(f, now))
        assertTrue(card(f).actions.isEmpty())
        assertEquals("Your Vault is kept on your server HQ, hq.example.com. This phone cannot see its backups yet.", card(f).body)
        val said = f.copy(serverReport = HqClient.ServerBackup(true, hoursAgo(3), false, false, 24))
        assertEquals("Your Vault is kept on your server HQ, hq.example.com. Last backup 3 h ago.", card(said).body)
        assertEquals("Your Vault is kept on your server HQ, hq.example.com. Its last backup did not finish.", card(said.copy(serverReport = HqClient.ServerBackup(true, null, true, false, 24))).body)
        assertEquals(BackupWords.Tone.Good, card(said).tone)
    }

    @Test fun theSlotHoldsTheFirstTwoSentencesAndOnlyWhenTheRowWarns() {
        val f = BackupFacts(on, newestMs = hoursAgo(60))
        val slot = BackupWords.slot(w, f, now)!!
        assertEquals("Check your backups", slot.title)
        assertEquals("Last backup 2 d ago. A new one should be made every 24 hours. Something is stopping it. Restart HQ, or copy details for whoever set up HQ.".substringBeforeLast(" Something"), slot.body.substringBeforeLast(" Something"))
        assertNull(BackupWords.slot(w, BackupFacts(on, newestMs = hoursAgo(1), copyOffPhoneMs = off), now))
    }

    @Test fun theRowWarnsExactlyWhenTheCardWarnsAndTheSlotShows() {
        val fixtures = listOf(
            BackupFacts(), BackupFacts(copyOffPhoneMs = off), BackupFacts(saveCopyOn = false), BackupFacts(server = true), BackupFacts(on, readable = false), BackupFacts(on, dirOutside = true),
            BackupFacts(on, BackupStatus(false, "x", 0, 0)), BackupFacts(on, firstSeenOnMs = hoursAgo(1)), BackupFacts(on, firstSeenOnMs = hoursAgo(60)),
            BackupFacts(on, newestMs = hoursAgo(60)), BackupFacts(on, newestMs = hoursAgo(1)), BackupFacts(on, newestMs = hoursAgo(1), copyOffPhoneMs = off),
        )
        fixtures.forEach { f ->
            val warns = BackupWords.warns(f, now)
            assertEquals(warns, card(f).tone == BackupWords.Tone.Warn)
            assertEquals(warns, BackupWords.slot(w, f, now) != null)
        }
    }

    @Test fun theDetailsHoldFactsAndNoKeysOrPaths() {
        val text = BackupWords.details(BackupFacts(on, newestMs = hoursAgo(60)), now)
        assertTrue(text.contains("state=Overdue"))
        assertFalse(text.contains("/k"))
        assertFalse(text.contains("/d"))
    }
}
