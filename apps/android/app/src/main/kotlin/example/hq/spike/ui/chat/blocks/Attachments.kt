package example.hq.spike.ui.chat.blocks

import org.json.JSONArray
import org.json.JSONException

/** A file saved in the vault for a chat message; [path] is vault-relative. */
data class Attachment(val name: String, val path: String, val mime: String, val size: Long) {
    /** "Word document", "Picture" and so on, from the file name; the mime type only when the name says nothing. */
    val kind: String get() = KINDS[name.substringAfterLast('.', "").lowercase()] ?: if (mime.startsWith("image/")) "Picture" else "File"

    val sizeWords: String get() = when {
        size < KB -> "$size B"
        size < MB -> "${Math.round(size / KB.toDouble())} KB"
        else -> "%.1f MB".format(java.util.Locale.US, size / MB.toDouble())
    }

    private companion object {
        const val KB = 1024L
        const val MB = KB * KB
        val KINDS = mapOf(
            "pdf" to "PDF document", "doc" to "Word document", "docx" to "Word document", "xls" to "Excel spreadsheet",
            "xlsx" to "Excel spreadsheet", "csv" to "CSV spreadsheet", "ppt" to "PowerPoint deck", "pptx" to "PowerPoint deck",
            "png" to "Picture", "jpg" to "Picture", "jpeg" to "Picture", "gif" to "Picture", "webp" to "Picture", "heic" to "Picture",
            "md" to "Markdown note", "txt" to "Text file", "json" to "JSON file", "zip" to "Zip archive", "html" to "Web page", "svg" to "Vector picture",
        )
    }
}

/** The server appends `<hq-attachments>[...]</hq-attachments>` to a saved message; the chat shows the files as cards, not as that text. */
object Attachments {
    /** The server keeps every chat upload under this folder; a path anywhere else is not one of its files. */
    const val UPLOAD_ROOT = "_media/web/"

    private val MARKER = Regex("\\s*<hq-attachments>([\\s\\S]*?)(?:</hq-attachments>|$)")

    fun split(content: String): Pair<String, List<Attachment>> {
        val m = MARKER.find(content) ?: return content to emptyList()
        val text = content.substring(0, m.range.first)
        return text to try {
            val arr = JSONArray(m.groupValues[1])
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                Attachment(o.getString("name"), o.getString("path"), o.optString("mime", ""), o.optLong("size", 0))
            }.filter { it.path.startsWith(UPLOAD_ROOT) && !it.path.contains("..") }
        } catch (e: JSONException) {
            emptyList()
        }
    }
}
