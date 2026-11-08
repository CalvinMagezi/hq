package example.hq.spike.ui.firstrun

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Checks 1, 3 (logic), 5, 6 and 7 of wave-0/03-data-transfer.md that run without a phone. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class MovingPhonesTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    private fun res(path: String) = File("src/main/$path").readText()

    private fun cleanSlate() {
        context.filesDir.resolve("hqhome").deleteRecursively()
        context.noBackupFilesDir.listFiles()?.forEach { it.deleteRecursively() }
        context.getSharedPreferences("firstrun", Context.MODE_PRIVATE).edit().clear().commit()
        context.getSharedPreferences("startup", Context.MODE_PRIVATE).edit().clear().commit()
        context.getSharedPreferences("secrets", Context.MODE_PRIVATE).edit().clear().commit()
    }

    private fun vault(): File = Moved.vault(context).also { File(it, "_data").mkdirs() }

    @Test fun cloudBackupTakesNothingAndTransferTakesOnlyTheVault() {
        val xml = res("res/xml/data_extraction_rules.xml")
        val cloud = xml.substringAfter("<cloud-backup>").substringBefore("</cloud-backup>")
        val transfer = xml.substringAfter("<device-transfer>").substringBefore("</device-transfer>")
        assertFalse(cloud.contains("<include"))
        listOf("root", "file", "database", "sharedpref", "external").forEach { assertTrue(cloud.contains("<exclude domain=\"$it\" path=\".\"/>")) }
        assertEquals(listOf("<include domain=\"file\" path=\"hqhome/.vault\"/>"), Regex("<include[^>]*/>").findAll(transfer).map { it.value }.toList())
        // Only the keys HQ keeps inside the vault stay behind.
        assertEquals(
            listOf(
                "<exclude domain=\"file\" path=\"hqhome/.vault/_data/connector_tls\"/>",
                "<exclude domain=\"file\" path=\"hqhome/.vault/_data/web_push_vapid.p8\"/>",
            ),
            Regex("<exclude[^>]*/>").findAll(transfer).map { it.value }.toList(),
        )
    }

    @Test fun theManifestPointsAtTheRulesAndKeepsAllowBackupOff() {
        val manifest = res("AndroidManifest.xml")
        assertTrue(manifest.contains("android:allowBackup=\"false\""))
        assertTrue(manifest.contains("android:dataExtractionRules=\"@xml/data_extraction_rules\""))
    }

    @Test fun movedNeedsAVaultAndNoRecordAndNoMarker() {
        assertTrue(Moved.isMoved(vaultExists = true, hasRecord = false, markerExists = false))
        assertFalse(Moved.isMoved(vaultExists = false, hasRecord = false, markerExists = false))
        assertFalse(Moved.isMoved(vaultExists = true, hasRecord = true, markerExists = false))
        assertFalse(Moved.isMoved(vaultExists = true, hasRecord = false, markerExists = true))
    }

    @Test fun aCopiedVaultOpensTheMovedStepAndKeepsItAcrossKills() {
        cleanSlate()
        vault()
        val store = FirstRunStore(context)
        assertEquals(Step.Moved, store.step())
        assertTrue(store.moved())
        assertEquals(Step.Moved, FirstRunStore(context).step())
        assertFalse(InstallMarker.exists(context))
    }

    @Test fun anUpgradeIsNeverTakenForMovedAndLeavesTheMarker() {
        cleanSlate()
        vault()
        context.getSharedPreferences("startup", Context.MODE_PRIVATE).edit().putBoolean("first_done", true).commit()
        assertNull(FirstRunStore(context).step())
        assertTrue(InstallMarker.exists(context))
        context.getSharedPreferences("startup", Context.MODE_PRIVATE).edit().clear().commit()
        assertNull(FirstRunStore(context).step())
    }

    @Test fun aFinishedFlowIsNeverMoved() {
        cleanSlate()
        vault()
        context.getSharedPreferences("firstrun", Context.MODE_PRIVATE).edit().putBoolean("done", true).commit()
        assertNull(FirstRunStore(context).step())
    }

    @Test fun aCleanInstallWritesTheMarkerBeforeTheVaultExists() {
        cleanSlate()
        assertNull(FirstRunStore(context).step())
        assertTrue(InstallMarker.file(context).startsWith(context.noBackupFilesDir))
        vault()
        assertNull(FirstRunStore(context).step())
    }

    @Test fun finishingAMovedFlowWritesTheMarkerAndForgetsTheMove() {
        cleanSlate()
        vault()
        val store = FirstRunStore(context)
        store.step()
        store.finish()
        assertFalse(store.moved())
        assertTrue(InstallMarker.exists(context))
        assertNull(FirstRunStore(context).step())
    }

    @Test fun startingFreshReturnsToACleanFirstRun() {
        cleanSlate()
        vault()
        val store = FirstRunStore(context)
        store.step()
        Moved.vault(context).deleteRecursively()
        store.forgetMoved()
        assertNull(store.step())
        assertFalse(store.moved())
    }

    @Test fun backFromTheKeyStepReturnsToMovedOnlyOnAMovedPhone() {
        assertEquals(Step.Moved, FirstRunLogic.back(Step.Model, moved = true))
        assertEquals(Step.Welcome, FirstRunLogic.back(Step.Model))
        assertEquals(Step.Model, FirstRunLogic.next(Step.Moved))
    }

    @Test fun theCommandAndItsEnvironmentCarryNoCredential() {
        assertEquals(listOf("/lib/libhq.so", "vault", "moved"), MovedPhone.args("/lib/libhq.so"))
        val env = MovedPhone.env("/h", "/t", "/h/config.yaml", "/h/.vault", "/root", "/data")
        assertEquals("/h/.vault", env["HQ_VAULT_PATH"])
        assertTrue(env.keys.none { it.endsWith("_KEY") || it.endsWith("TOKEN") })
    }

    @Test fun theServiceIsNamedFromTheBackendNameOnly() {
        assertEquals("OpenRouter", MovedPhone.serviceFor("openrouter/some-model"))
        assertEquals("Anthropic", MovedPhone.serviceFor("Anthropic"))
        assertNull(MovedPhone.serviceFor("https://token@example.com/v1"))
        assertNull(MovedPhone.serviceFor(null))
    }

    @Test fun aCleanDatabaseGivesCountsAndABrokenOneIsCaught() {
        cleanSlate()
        val v = vault()
        File(v, "a.md").writeText("x")
        File(v, "Notes").mkdirs()
        File(v, "Notes/b.md").writeText("y")
        File(v, "_data/skip.md").writeText("z")
        val db = File(v, MovedPhone.DB)
        android.database.sqlite.SQLiteDatabase.openOrCreateDatabase(db, null).use {
            it.execSQL("CREATE TABLE tasks (id INTEGER)")
            it.execSQL("INSERT INTO tasks VALUES (1), (2), (3)")
        }
        val facts = MovedPhone.facts(v)
        assertTrue(facts.databaseOk)
        assertEquals(2, facts.notes)
        assertEquals(3, facts.tasks)
        db.writeBytes(ByteArray(4096) { 7 })
        assertFalse(MovedPhone.facts(v).databaseOk)
    }

    @Test fun notesOnlyKeepsTheNotesAndDropsTheDatabaseFiles() {
        cleanSlate()
        val v = vault()
        File(v, "a.md").writeText("x")
        listOf(MovedPhone.DB, "${MovedPhone.DB}-wal", "${MovedPhone.DB}-shm").forEach { File(v, it).writeText("junk") }
        MovedPhone.dropDatabase(v)
        assertTrue(File(v, "a.md").exists())
        assertFalse(File(v, MovedPhone.DB).exists())
        assertFalse(File(v, "${MovedPhone.DB}-wal").exists())
        assertTrue(MovedPhone.databaseOk(File(v, MovedPhone.DB)))
    }
}
