package example.hq.spike.ui.firstrun

import java.io.File
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** Restore opens a copy saved by safe mode: no secret, and a name that climbs out of the folder is refused. */
class RestoreZipTest {
    @get:Rule val tmp = TemporaryFolder()

    private fun zip(vararg entries: Pair<String, String>): File = tmp.newFile().also { f ->
        ZipOutputStream(f.outputStream()).use { z -> entries.forEach { (n, t) -> z.putNextEntry(ZipEntry(n)); z.write(t.toByteArray()); z.closeEntry() } }
    }

    @Test fun zipsAreToldApartByTheirFirstBytes() {
        assertTrue(RestoreLogic.isZip(zip("a" to "b")))
        assertFalse(RestoreLogic.isZip(tmp.newFile().also { it.writeText("HQBAK-not-a-zip") }))
        assertFalse(RestoreLogic.isZip(tmp.newFile().also { it.writeText("PK") }))
    }

    @Test fun unzipWritesTheFilesAndFolders() {
        val into = File(tmp.root, "out")
        RestoreLogic.unzip(zip("Notes/a.md" to "hi", "_data/vault.db" to "db"), into)
        assertEquals("hi", File(into, "Notes/a.md").readText())
        assertTrue(File(into, "_data/vault.db").isFile)
    }

    @Test fun aNameThatClimbsOutIsRefusedAndWritesNothingOutside() {
        val into = File(tmp.root, "out2")
        val r = runCatching { RestoreLogic.unzip(zip("../evil.txt" to "x"), into) }
        assertTrue(r.exceptionOrNull() is java.io.IOException)
        assertFalse(File(tmp.root, "evil.txt").exists())
    }

    @Test fun aCopyThatWouldNotFitIsRefusedBeforeAnythingIsWritten() {
        val into = File(tmp.root, "out3")
        val r = runCatching { RestoreLogic.unzip(zip("Notes/a.md" to "x".repeat(4096)), into, spaceLeft = RestoreLogic.SPACE_MARGIN_BYTES + 100) }
        assertTrue(r.exceptionOrNull() is java.io.IOException)
        assertFalse(File(into, "Notes/a.md").exists())
    }
}
