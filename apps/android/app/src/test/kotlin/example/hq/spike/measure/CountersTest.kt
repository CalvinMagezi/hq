package example.hq.spike.measure

import java.io.File
import java.nio.file.Files
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CountersTest {
    private val dir: File = Files.createTempDirectory("counters").toFile().also { it.deleteOnExit() }
    private val file = File(dir, Counters.FILE)
    private var day = "2026-11-03"

    private fun open(version: String = "0.9.1") = Counters(file, version) { day }

    private fun bumpEverything(c: Counters) = Counter.entries.forEach { counter -> (0 until counter.slots).forEach { c.bump(counter, it, 3) } }

    @Test fun bumpsAddUpAndSurviveAReopen() {
        val c = open()
        c.bump(Counter.LockedUnlocked)
        c.bump(Counter.LockedUnlocked)
        c.bump(Counter.UndoTaskDiscarded, 1)
        c.bump(Counter.UndoTaskDiscarded, 0)
        val again = open().snapshot().current
        assertEquals(2, again.slot(Counter.LockedUnlocked))
        assertEquals(1, again.slot(Counter.UndoTaskDiscarded, 0))
        assertEquals(1, again.slot(Counter.UndoTaskDiscarded, 1))
    }

    @Test fun bytesSplitByNetwork() {
        val c = open()
        c.addBytes(DataPurpose.SpeechModel, metered = false, n = 57_400_000)
        c.addBytes(DataPurpose.SpeechModel, metered = true, n = 1_000_000)
        val col = c.snapshot().current
        assertEquals(1_000_000, col.slot(Counter.DataSpeechModel, 0))
        assertEquals(57_400_000, col.slot(Counter.DataSpeechModel, 1))
        assertEquals(57, Noticed.mb(col.slot(Counter.DataSpeechModel, 1)))
    }

    @Test fun aSlotOutsideTheCounterAndNonsenseAreIgnored() {
        val c = open()
        c.bump(Counter.LockedUnlocked, slot = 1)
        c.bump(Counter.LockedUnlocked, by = -5)
        c.bump(Counter.LockedUnlocked, by = 0)
        assertEquals(0, c.snapshot().current.slot(Counter.LockedUnlocked))
    }

    @Test fun keysInTheFileComeOnlyFromTheEnum() {
        val c = open()
        bumpEverything(c)
        val keys = Counter.entries.map { it.key }
        assertEquals(keys.size, keys.toSet().size)
        val counts = JSONObject(file.readText()).getJSONObject("current").getJSONObject("c")
        assertEquals(keys.toSet(), counts.keys().asSequence().toSet())
    }

    @Test fun anUnknownKeyInTheFileIsDropped() {
        file.writeText("""{"on":true,"current":{"v":"0.9.1","d":"2026-11-03","c":{"note.title":[7],"locked_press.unlocked":[4]}}}""")
        val col = open().snapshot().current
        assertEquals(4, col.slot(Counter.LockedUnlocked))
        assertEquals(setOf(Counter.LockedUnlocked), col.values.keys)
    }

    @Test fun everyNumberInTheFileIsAWholeNumber() {
        bumpEverything(open())
        fun walk(v: Any?) {
            when (v) {
                is JSONObject -> v.keys().forEach { walk(v.get(it)) }
                is JSONArray -> (0 until v.length()).forEach { walk(v.get(it)) }
                is Number -> assertTrue("$v", v is Int || v is Long)
            }
        }
        walk(JSONObject(file.readText()))
    }

    @Test fun aFractionOrANegativeInTheFileReadsAsZero() {
        file.writeText("""{"on":true,"current":{"v":"0.9.1","d":"2026-11-03","c":{"data_mb.other":[1.5,-4]}}}""")
        val col = open().snapshot().current
        assertEquals(0, col.slot(Counter.DataOther, 0))
        assertEquals(0, col.slot(Counter.DataOther, 1))
    }

    @Test fun theFileHoldsNoStringsButTheVersionAndTheDate() {
        val c = open()
        bumpEverything(c)
        open("0.9.2")
        c.setOn(false)
        val strings = mutableListOf<Pair<String, String>>()
        fun walk(name: String, v: Any?) {
            when (v) {
                is JSONObject -> v.keys().forEach { walk(it, v.get(it)) }
                is JSONArray -> (0 until v.length()).forEach { walk(name, v.get(it)) }
                is String -> strings += name to v
            }
        }
        val root = JSONObject(file.readText())
        root.keys().forEach { k -> if (k != "c") walk(k, root.get(k)) }
        assertTrue(strings.isNotEmpty())
        val ok = Regex("""\d+\.\d+\.\d+|\d{4}-\d{2}-\d{2}|""")
        strings.forEach { (_, value) -> assertTrue(value, ok.matches(value)) }
    }

    @Test fun theFileNeverPassesTheCap() {
        val c = open()
        repeat(3) { bumpEverything(c); open("0.9.${it + 2}") }
        c.bump(Counter.DataOther, 0, 999_999_999_999)
        assertTrue(file.length() <= Counters.MAX_BYTES)
    }

    @Test fun aFileOverTheCapIsNotReadAndStartsAtZero() {
        file.writeText(""" {"on":false,"current":{"v":"0.9.1","d":"2026-11-03","c":{"locked_press.unlocked":[4]}}}""" + " ".repeat(Counters.MAX_BYTES))
        val s = open().snapshot()
        assertTrue(s.on)
        assertEquals(0, s.current.slot(Counter.LockedUnlocked))
    }

    @Test fun anUnreadableFileStartsAtZero() {
        file.writeText("not json {{{")
        val s = open().snapshot()
        assertEquals(0, s.current.slot(Counter.LockedUnlocked))
        assertNull(s.previous)
    }

    @Test fun aWriteLeavesNoTemporaryFileBehind() {
        open().bump(Counter.NoticePosted)
        assertEquals(listOf(Counters.FILE), dir.list()!!.toList())
    }

    @Test fun aNewVersionMovesThisColumnToThePreviousOne() {
        val first = open("0.9.0")
        first.bump(Counter.NoticePosted)
        first.bump(Counter.NoticePosted)
        day = "2026-11-05"
        val second = open("0.9.1")
        val s = second.snapshot()
        assertEquals("0.9.1", s.current.version)
        assertEquals("2026-11-05", s.current.since)
        assertEquals(0, s.current.slot(Counter.NoticePosted))
        assertEquals("0.9.0", s.previous!!.version)
        assertEquals(2, s.previous!!.slot(Counter.NoticePosted))
        second.bump(Counter.NoticePosted)
        assertEquals(1, open("0.9.1").snapshot().current.slot(Counter.NoticePosted))
    }

    @Test fun onlyOnePreviousVersionIsKept() {
        open("0.9.0").bump(Counter.NoticePosted)
        open("0.9.1").bump(Counter.NoticeTapped)
        val s = open("0.9.2").snapshot()
        assertEquals("0.9.1", s.previous!!.version)
        assertEquals(0, s.previous!!.slot(Counter.NoticePosted))
        assertEquals(1, s.previous!!.slot(Counter.NoticeTapped))
    }

    @Test fun countingOffStopsEverythingAndKeepsTheNumbers() {
        val c = open()
        c.bump(Counter.NoticePosted)
        day = "2026-11-05"
        c.setOn(false)
        c.bump(Counter.NoticePosted)
        c.set(Counter.AssistantChosen, 1)
        c.addBytes(DataPurpose.AppUpdates, true, 5_000_000)
        val s = open().snapshot()
        assertFalse(s.on)
        assertEquals("2026-11-05", s.stoppedOn)
        assertEquals(1, s.current.slot(Counter.NoticePosted))
        assertEquals(0, s.current.slot(Counter.AssistantChosen))
        assertEquals(0, s.current.slot(Counter.DataAppUpdates))
        c.setOn(true)
        c.bump(Counter.NoticePosted)
        assertEquals(2, c.snapshot().current.slot(Counter.NoticePosted))
    }

    @Test fun clearingZeroesBothColumnsAndCountsFromNow() {
        open("0.9.0").bump(Counter.NoticePosted)
        val c = open("0.9.1")
        c.bump(Counter.NoticePosted)
        day = "2026-11-09"
        c.clear()
        val s = open("0.9.1").snapshot()
        assertEquals(0, s.current.slot(Counter.NoticePosted))
        assertEquals("2026-11-09", s.current.since)
        assertNull(s.previous)
    }

    @Test fun theAssistantIsAStateNotACount() {
        val c = open()
        c.set(Counter.AssistantChosen, 1)
        c.set(Counter.AssistantChosen, 1)
        assertEquals(1, c.snapshot().current.slot(Counter.AssistantChosen))
        c.set(Counter.AssistantChosen, 0)
        assertEquals(0, c.snapshot().current.slot(Counter.AssistantChosen))
    }

    private fun sources(vararg folders: String): List<File> =
        folders.flatMap { File("src/main/kotlin/example/hq/spike/$it").walkTopDown().filter { f -> f.extension == "kt" }.toList() }

    @Test fun theMeasurePackageCannotReachTheNetwork() {
        val files = sources("measure")
        assertTrue(files.size >= 2)
        files.forEach { f ->
            f.readLines().filter { it.trimStart().startsWith("import ") }.forEach { line ->
                assertFalse("${f.name}: $line", line.contains("java.net") || line.contains("okhttp3") || line.contains("javax.net"))
            }
        }
    }

    @Test fun nothingThatTalksToTheNetworkReadsTheCounters() {
        val files = sources("data", "update", "bridge") + File("src/main/java/example/hq/spike/Alerts.java")
        files.forEach { f -> assertFalse(f.name, f.readText().contains(".snapshot(")) }
    }
}
