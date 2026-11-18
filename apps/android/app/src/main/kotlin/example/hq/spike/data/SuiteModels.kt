package example.hq.spike.data

import java.io.IOException
import org.json.JSONObject

/** Where one office file stands for the suite, as `/api/suite/file` words it. */
data class SuiteFile(
    val path: String,
    /** `ready`, `prepared` or `shrunk`. */
    val state: String,
    val job: String?,
    val note: String?,
    val savedBytes: Long?,
    /** One sentence, written by hq. */
    val headline: String,
    /** The lines that go with a question, written by hq. Empty unless hq is waiting for an answer. */
    val details: List<String>,
    val canShrink: Boolean,
    val canConfirm: Boolean,
    val canAbandon: Boolean,
    val canUndo: Boolean,
)

/** hq looked at the file and said no, or could not do it; [message] is a sentence for the person. */
class SuiteRefused(message: String) : IOException(message)

internal object SuiteWire {
    fun file(o: JSONObject): SuiteFile {
        val actions = o.optJSONObject("actions") ?: JSONObject()
        val lines = o.optJSONArray("details")
        return SuiteFile(
            path = o.optString("path", ""),
            state = o.optString("state", "ready"),
            job = NoteWire.text(o, "job"),
            note = NoteWire.text(o, "note"),
            savedBytes = if (o.has("saved_bytes") && !o.isNull("saved_bytes")) o.optLong("saved_bytes") else null,
            headline = o.optString("headline", ""),
            details = if (lines == null) emptyList() else (0 until lines.length()).mapNotNull { (lines.opt(it) as? String)?.takeIf(String::isNotBlank) },
            canShrink = actions.optBoolean("shrink", false),
            canConfirm = actions.optBoolean("confirm", false),
            canAbandon = actions.optBoolean("abandon", false),
            canUndo = actions.optBoolean("undo", false),
        )
    }

    /** The sentence hq put in an error body, if it did. */
    fun refusal(body: String): String? = runCatching { JSONObject(body).optString("error", "") }.getOrNull()?.takeIf { it.isNotBlank() }
}

/** One calculated cell of a sheet, as `/api/suite/sheet` words it. */
data class SheetCell(val text: String, val formula: Boolean, val number: Boolean, val error: Boolean, val raw: String = "")

/** A bounded window of a `sheet` block, already calculated and formatted by hq. */
data class SheetView(val name: String, val rows: List<List<SheetCell>>, val row0: Int, val col0: Int, val rowCount: Int, val colCount: Int, val text: String)

internal object SheetWire {
    fun view(o: JSONObject): SheetView {
        val rows = o.optJSONArray("rows")
        return SheetView(
            name = o.optString("name", "Sheet"),
            rows = if (rows == null) emptyList() else (0 until rows.length()).map { r ->
                val row = rows.optJSONArray(r)
                if (row == null) emptyList() else (0 until row.length()).map { c ->
                    val cell = row.optJSONObject(c) ?: JSONObject()
                    SheetCell(cell.optString("text", ""), cell.optBoolean("formula", false), cell.optBoolean("number", false), cell.optBoolean("error", false), cell.optString("raw", ""))
                }
            },
            row0 = o.optInt("row0", 0), col0 = o.optInt("col0", 0),
            rowCount = o.optInt("row_count", 0), colCount = o.optInt("col_count", 0),
            text = o.optString("text", ""),
        )
    }
}

/** One computed number of a `tile` block, formatted by hq. */
data class TileView(val label: String, val text: String)

internal fun tileView(o: JSONObject) = TileView(o.optString("label", ""), o.optString("text", ""))

/** A cold-storage sink as the phone lists it. */
data class SinkInfo(val id: String, val kind: String, val place: String, val reachable: Boolean)

/** What checking a sink found: the words hq wrote, and how many stored originals could not be restored. */
data class SinkCheck(val text: String, val failed: Int)

internal object SinkWire {
    fun list(o: JSONObject): List<SinkInfo> {
        val a = o.optJSONArray("sinks") ?: return emptyList()
        return (0 until a.length()).mapNotNull { a.optJSONObject(it) }.map {
            SinkInfo(it.optString("id", ""), it.optString("kind", "folder"), it.optString("place", ""), it.optBoolean("reachable", false))
        }
    }
}
