package example.hq.spike.safe

import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.file.Files
import java.time.LocalDateTime
import java.util.zip.ZipFile
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** Checks of wave-0/02-safe-mode.md that need no phone: what goes in the copy, the zip itself, space, failures, cancel and the crash markers. */
class NotesCopyTest {
    @get:Rule val tmp = TemporaryFolder()

    private fun phone(): File = tmp.newFolder("files").also {
        val v = File(it, NotesCopy.VAULT)
        File(v, "Notes").mkdirs(); File(v, "_data").mkdirs(); File(v, "_logs").mkdirs(); File(v, ".hq-restore-1").mkdirs()
        File(v, "Notes/a.md").writeText("hello ".repeat(100))
        File(v, "Notes/pic.png").writeBytes(ByteArray(300) { it.toByte() })
        File(v, "_data/vault.db").writeText("db")
        File(v, "_data/vault.db-wal").writeText("wal")
        File(v, "_data/vault.db-shm").writeText("shm")
        File(v, "_logs/x.log").writeText("log")
        File(v, "Notes/hq.lock").writeText("lock")
        File(v, ".hq-restore-1/y.md").writeText("half")
        File(v, "bridge.key").writeText("key")
        File(v, "config.yaml").writeText("cfg")
        File(v, "Notes/old.hqbak").writeText("bak")
        File(v, "_data/connector_tls").mkdirs()
        File(v, "_data/connector_tls/key.pem").writeText("tls key")
        File(v, "_data/web_push_vapid.p8").writeText("push key")
    }

    @Test fun excludesLogsLocksKeysConfigAndRestoreFolders() {
        // The connector TLS keys and the web push key stay on the phone, like the bridge key.
        val names = NotesCopy.plan(phone()).entries.map { it.name }
        assertEquals(listOf("Notes/a.md", "Notes/pic.png", "_data/vault.db", "_data/vault.db-shm", "_data/vault.db-wal").sorted(), names.sorted())
    }

    @Test fun databaseFilesComeFirst() {
        val p = NotesCopy.plan(phone())
        assertEquals(3, p.databaseFiles)
        assertTrue(p.entries.take(3).all { NotesCopy.isDatabase(it.name) })
    }

    @Test fun symlinksAreSkipped() {
        val d = phone()
        val outside = tmp.newFile("secret.txt")
        Files.createSymbolicLink(File(d, NotesCopy.VAULT + "/Notes/link.md").toPath(), outside.toPath())
        assertFalse(NotesCopy.plan(d).entries.any { it.name.endsWith("link.md") })
    }

    @Test fun waitingNotesBecomeAFileAndAreCounted() {
        val d = phone()
        File(d, NotesCopy.OUTBOX).writeText("""{"items":[{"id":"1","kind":"note","title":"Milk","payload":{"folder":"Inbox","title":"Milk","content":"buy milk"}}]}""")
        val p = NotesCopy.plan(d)
        assertEquals(1, p.waiting)
        val e = p.entries.first { it.name == NotesCopy.WAITING_NAME }
        assertTrue(String(e.text!!).contains("buy milk"))
        assertEquals(1, NotesCopy.waitingCount(d))
    }

    @Test fun serverOnlyPhoneCopiesJustTheWaitingNotes() {
        val d = tmp.newFolder("srv")
        File(d, NotesCopy.OUTBOX).writeText("""{"items":[{"kind":"task","title":"Call","payload":{}}]}""")
        val p = NotesCopy.plan(d, withVault = false)
        assertEquals(listOf(NotesCopy.WAITING_NAME), p.entries.map { it.name })
    }

    @Test fun brokenOutboxIsIgnored() {
        val d = phone()
        File(d, NotesCopy.OUTBOX).writeText("not json")
        assertEquals(0, NotesCopy.plan(d).waiting)
    }

    @Test fun fileNameFollowsThePattern() {
        assertEquals("HQ Vault 2026-05-04 0907.zip", NotesCopy.fileName(LocalDateTime.of(2026, 5, 4, 9, 7)))
    }

    @Test fun zipHoldsEveryFileWithItsBytesAndPackedTypesAreStored() {
        val d = phone()
        val out = ByteArrayOutputStream()
        val r = NotesCopy.write(NotesCopy.plan(d), out)
        assertEquals(5, r.files)
        val f = tmp.newFile("out.zip").also { it.writeBytes(out.toByteArray()) }
        ZipFile(f).use { z ->
            assertEquals("hello ".repeat(100), z.getInputStream(z.getEntry("Notes/a.md")).readBytes().decodeToString())
            assertEquals(300, z.getInputStream(z.getEntry("Notes/pic.png")).readBytes().size)
            assertTrue(z.getEntry("Notes/pic.png").compressedSize >= 300)
        }
        assertTrue(r.leftOut.isEmpty())
    }

    @Test fun unreadableFileIsLeftOutAndNamed() {
        val d = phone()
        val p = NotesCopy.plan(d)
        File(d, NotesCopy.VAULT + "/Notes/a.md").delete()
        val out = ByteArrayOutputStream()
        val r = NotesCopy.write(p, out)
        assertEquals(listOf("Notes/a.md"), r.leftOut)
        val f = tmp.newFile("o2.zip").also { it.writeBytes(out.toByteArray()) }
        ZipFile(f).use { z -> assertTrue(z.getInputStream(z.getEntry(NotesCopy.LEFT_OUT_NAME)).readBytes().decodeToString().contains("Notes/a.md")) }
    }

    @Test fun cancelStopsTheCopy() {
        var n = 0
        val failed = runCatching { NotesCopy.write(NotesCopy.plan(phone()), ByteArrayOutputStream(), cancelled = { n++ > 1 }) }.exceptionOrNull()
        assertTrue(failed is NotesCopy.Cancelled)
    }

    @Test fun databaseCallbackFiresOnceAfterItsLastFile() {
        var calls = 0
        NotesCopy.write(NotesCopy.plan(phone()), ByteArrayOutputStream(), onDatabaseCopied = { calls++ })
        assertEquals(1, calls)
    }

    @Test fun progressIsThrottledButEndsComplete() {
        val seen = ArrayList<NotesCopy.Progress>()
        var t = 0L
        NotesCopy.write(NotesCopy.plan(phone()), ByteArrayOutputStream(), now = { t += 10; t }) { seen += it }
        assertTrue(seen.size < 30)
        assertEquals(5, seen.last().filesDone)
    }

    @Test fun spaceCheckNeedsSizePlusMargin() {
        val p = NotesCopy.plan(phone())
        assertTrue(NotesCopy.short(p, 1000))
        assertFalse(NotesCopy.short(p, p.bytes + NotesCopy.SPACE_MARGIN))
        assertFalse(NotesCopy.short(p, null))
        assertEquals(NotesCopy.SPACE_MARGIN + p.bytes - 1000, NotesCopy.freeUpBytes(p, 1000))
    }

    @Test fun failuresAreClassed() {
        assertEquals(NotesCopy.Failure.PlaceFull, NotesCopy.classify(java.io.IOException("write failed: ENOSPC (No space left on device)")))
        assertEquals(NotesCopy.Failure.PlaceFull, NotesCopy.classify(java.io.IOException("x", java.io.IOException("No space left on device"))))
        assertEquals(NotesCopy.Failure.PlaceGone, NotesCopy.classify(java.io.IOException("Broken pipe")))
    }

    @Test fun timeLeftAndSizes() {
        assertNull(NotesCopy.secondsLeft(NotesCopy.Progress(0, 1, 0, 100, 5000)))
        assertEquals(10L, NotesCopy.secondsLeft(NotesCopy.Progress(1, 2, 50, 100, 10_000)))
        assertEquals("340 MB", NotesCopy.size(340_000_000))
        assertEquals("1.2 GB", NotesCopy.size(1_200_000_000))
        assertEquals("212 kB", NotesCopy.size(212_000))
    }

    @Test fun pendingAndLastCopyMarkers() {
        val d = tmp.newFolder("m")
        assertNull(NotesCopy.pending(d))
        NotesCopy.markPending(d, "content://x/1")
        assertEquals("content://x/1", NotesCopy.pending(d))
        NotesCopy.clearPending(d)
        assertNull(NotesCopy.pending(d))
        NotesCopy.recordCopy(d, 42)
        assertEquals(42L, NotesCopy.lastCopy(d))
    }

    @Test fun twoCrashedLaunchesSendTheThirdToSafeMode() {
        val d = tmp.newFolder("c")
        assertFalse(CrashMarker.begin(d))
        assertFalse(CrashMarker.begin(d))
        assertTrue(CrashMarker.begin(d))
        assertTrue(CrashMarker.tripped(d))
        CrashMarker.reset(d)
        assertFalse(CrashMarker.tripped(d))
    }

    @Test fun aLaunchThatDrewAFrameClearsTheCount() {
        val d = tmp.newFolder("c2")
        CrashMarker.begin(d); CrashMarker.begin(d)
        CrashMarker.drawn(d)
        assertFalse(CrashMarker.begin(d))
        assertFalse(CrashMarker.tripped(d))
    }

    @Test fun safeScreenUsesCatalogueWordsAndNoHexColours() {
        val src = File("src/main/kotlin/example/hq/spike/safe/SafeModeActivity.kt").readText()
        assertFalse(Regex("0x[0-9A-Fa-f]{6,8}|\"#[0-9A-Fa-f]{6}\"").containsMatchIn(src))
        assertFalse(src.contains("\u2014"))
        assertFalse(src.contains("getIntent") || src.contains("intent.getStringExtra") || src.contains("intent.extras"))
        assertTrue(File("src/main/AndroidManifest.xml").readText().contains("android:process=\":safe\""))
        assertTrue(File("src/main/res/xml/shortcuts.xml").readText().contains("save_notes"))
    }
}
