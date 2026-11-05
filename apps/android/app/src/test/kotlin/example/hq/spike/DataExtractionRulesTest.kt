package example.hq.spike

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The backup rules: the cloud takes nothing, a phone-to-phone copy takes the vault only (not the saved copy, the outbox, keys or the bridge key). */
class DataExtractionRulesTest {
    private val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(File("src/main/res/xml/data_extraction_rules.xml"))

    private fun rules(section: String, tag: String) = (doc.getElementsByTagName(section).item(0) as org.w3c.dom.Element)
        .getElementsByTagName(tag).let { l -> (0 until l.length).map { (l.item(it) as org.w3c.dom.Element).let { e -> e.getAttribute("domain") to e.getAttribute("path") } } }

    @Test fun cloudBackupExcludesEveryDomainAndIncludesNothing() {
        assertEquals(listOf("root", "file", "database", "sharedpref", "external"), rules("cloud-backup", "exclude").map { it.first })
        assertTrue(rules("cloud-backup", "exclude").all { it.second == "." })
        assertTrue(rules("cloud-backup", "include").isEmpty())
    }

    @Test fun deviceTransferTakesOnlyTheVaultFolder() {
        assertEquals(listOf("file" to "hqhome/.vault"), rules("device-transfer", "include"))
        assertTrue(rules("device-transfer", "exclude").isEmpty())
    }

    @Test fun theManifestPointsAtTheRulesAndTurnsOldBackupOff() {
        val manifest = File("src/main/AndroidManifest.xml").readText()
        assertTrue(manifest.contains("@xml/data_extraction_rules"))
        assertTrue(manifest.contains("allowBackup=\"false\"") || manifest.contains("fullBackupContent"))
    }
}
