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
