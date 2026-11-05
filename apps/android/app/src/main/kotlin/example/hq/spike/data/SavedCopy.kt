package example.hq.spike.data

import android.content.Context
import java.io.File
import org.json.JSONArray
import org.json.JSONObject

/**
 * What the phone keeps of an answer so a screen can still show it when HQ cannot answer: one small JSON file per place, with the time HQ
 * answered and where the answer came from. A copy from a server HQ is removed the moment that server signs the phone out or is disconnected.
 */
class SavedCopy(private val dir: File) {
    enum class Source { Phone, Server }

    data class Saved(val place: String, val source: Source, val atMs: Long, val body: JSONObject)

    /**
     * Keeps [body] for [place]. A list under [listKey] that makes the file too big is cut to its first rows, and the oldest other copies
     * go when the folder passes its total, so the copy never grows without end.
     */
    @Synchronized
    fun write(place: String, source: Source, body: JSONObject, atMs: Long = System.currentTimeMillis(), listKey: String? = null) {
        require(PLACE.matches(place)) { "not a place name" }
        val rows = listKey?.let { body.optJSONArray(it) }
        var keep = rows?.length() ?: 0
        var text = wrap(source, atMs, body)
        while (rows != null && text.toByteArray().size > MAX_PLACE_BYTES && keep > 0) {
            keep /= 2
            body.put(listKey, JSONArray((0 until keep).map { rows.get(it) }))
            text = wrap(source, atMs, body)
        }
        if (text.toByteArray().size > MAX_PLACE_BYTES) return
        dir.mkdirs()
        val temp = File(dir, "$place.tmp")
        temp.writeText(text)
        // A rename inside one folder is atomic, so a reader sees the old copy or the new one, never half of either.
        if (!temp.renameTo(file(place))) { temp.delete(); return }
        trim(place)
    }

    @Synchronized
    fun read(place: String): Saved? {
        if (!PLACE.matches(place)) return null
        val f = file(place)
        if (!f.isFile) return null
        return try {
            val o = JSONObject(f.readText())
            Saved(place, Source.valueOf(o.getString("source")), o.getLong("at"), o.getJSONObject("body"))
        } catch (e: Exception) {
            f.delete()
            null
        }
    }

    @Synchronized
    fun forget(place: String) { if (PLACE.matches(place)) file(place).delete() }

    /** The server HQ signed the phone out, or it was disconnected: nothing that came from it stays. */
    @Synchronized
    fun clearServer() {
        files().forEach { f -> if (read(f.nameWithoutExtension)?.source == Source.Server) f.delete() }
    }

    @Synchronized
    fun clearAll() { files().forEach { it.delete() } }

    @Synchronized
    fun totalBytes(): Long = files().sumOf { it.length() }

    private fun wrap(source: Source, atMs: Long, body: JSONObject) = JSONObject().put("source", source.name).put("at", atMs).put("body", body).toString()

    private fun file(place: String) = File(dir, "$place.json")

    private fun files(): List<File> = dir.listFiles { f -> f.isFile && f.name.endsWith(".json") }?.toList().orEmpty()

    private fun trim(newest: String) {
        val others = files().filter { it.nameWithoutExtension != newest }.sortedBy { it.lastModified() }.toMutableList()
        var total = totalBytes()
        while (total > MAX_TOTAL_BYTES && others.isNotEmpty()) {
            val oldest = others.removeAt(0)
            total -= oldest.length()
            oldest.delete()
        }
    }

    companion object {
        const val MAX_PLACE_BYTES = 256 * 1024
        const val MAX_TOTAL_BYTES = 2L * 1024 * 1024
        private val PLACE = Regex("[a-z][a-z0-9_]{0,40}")

        fun of(context: Context) = SavedCopy(File(context.applicationContext.filesDir, "savedcopy"))
    }
}
