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
    /** The original is sealed in a sink: bringing it back needs that sink's recovery code. */
    val originalInSink: Boolean = false,
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
            originalInSink = o.optBoolean("original_in_sink", false),
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

/** One block of a note as the block editor lists it: its place, the hash hq checks an edit against, its kind and its markdown. */
data class BlockRow(val index: Int, val hash: String, val kind: String, val src: String)

internal object BlocksWire {
    fun list(o: JSONObject): List<BlockRow> {
        val a = o.optJSONArray("blocks") ?: return emptyList()
        return (0 until a.length()).mapNotNull { a.optJSONObject(it) }.map {
            BlockRow(it.optInt("index", 0), it.optString("hash", ""), it.optString("kind", ""), it.optString("src", ""))
        }
    }
}

/** One card of a slides note as `/api/suite/cards` lists it. [hash] is quoted back with every edit of the card. */
data class CardRow(
    val index: Int,
    val title: String,
    val hash: String,
    val blocks: Int,
    val layout: String,
    val transition: String?,
    val background: String?,
    val color: String?,
    val notes: List<String>,
)

internal object CardsWire {
    fun list(o: JSONObject): List<CardRow> {
        val a = o.optJSONArray("cards") ?: return emptyList()
        return (0 until a.length()).mapNotNull { a.optJSONObject(it) }.map { card(it) }
    }

    private fun card(c: JSONObject): CardRow {
        val notes = c.optJSONArray("notes")
        return CardRow(
            index = c.optInt("index", 0), title = c.optString("title", ""), hash = c.optString("hash", ""), blocks = c.optInt("blocks", 0),
            layout = c.optString("layout", "content"), transition = NoteWire.text(c, "transition"), background = NoteWire.text(c, "background"),
            color = NoteWire.text(c, "color"),
            notes = if (notes == null) emptyList() else (0 until notes.length()).mapNotNull { (notes.opt(it) as? String)?.takeIf(String::isNotBlank) },
        )
    }
}

/** One review comment as the `comments` action lists it. [on] is the hash of the block it is about. */
data class CommentRow(
    val index: Int,
    val hash: String,
    val on: String,
    val by: String?,
    val at: String?,
    val open: Boolean,
    val text: String,
    val orphaned: Boolean,
    /** The first line of the block it is about, when that block still exists. */
    val anchor: String?,
)

internal object CommentsWire {
    fun list(o: JSONObject): List<CommentRow> {
        val a = o.optJSONArray("comments") ?: return emptyList()
        return (0 until a.length()).mapNotNull { a.optJSONObject(it) }.map {
            CommentRow(
                index = it.optInt("index", 0), hash = it.optString("hash", ""), on = it.optString("on", ""), by = NoteWire.text(it, "by"), at = NoteWire.text(it, "at"),
                open = it.optString("state", "open") != "resolved", text = it.optString("text", ""), orphaned = it.optBoolean("orphaned", false),
                anchor = NoteWire.text(it, "anchor_preview"),
            )
        }
    }
}
