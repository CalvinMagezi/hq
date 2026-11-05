package example.hq.spike.safe

import java.io.File
import java.io.IOException
import java.nio.file.Files
import java.time.LocalDateTime
import java.time.format.DateTimeFormatter
import java.util.zip.Deflater
import org.json.JSONException
import org.json.JSONObject

/**
 * The one way the app copies the owner's notes into a .zip: used by the safe-mode screen (its own process, with HQ possibly dead)
 * and by Backup's "Save a copy now". Plain Kotlin and the JDK, plus org.json for the outbox file; nothing here loads Compose,
 * calls HQ or opens the network. [plan] decides what goes in and is pure over a folder, so tests run it on a fixture.
 */
object NotesCopy {
    /** Where the vault lives under the app's files folder. */
    const val VAULT = "hqhome/.vault"
    const val WAITING_NAME = "waiting-to-send.md"
    const val LEFT_OUT_NAME = "left-out.txt"
    const val OUTBOX = "outbox.json"

    /** Free space needed on top of the files themselves (the index and some slack). */
    const val SPACE_MARGIN = 10L * 1024 * 1024

    /** The most the hub holds HQ stopped for the database files. */
    const val HOLD_MAX_MS = 25_000L
    const val PROGRESS_EVERY_MS = 200L

    private const val COUNT_EVERY = 50
    private const val PENDING = "safe-copy.pending"
    private const val LAST_COPY = "safe-copy.last"

    private val PACKED = setOf("png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "avif", "pdf", "zip", "gz", "mp3", "m4a", "aac", "ogg", "opus", "mp4", "mov", "webm", "mkv", "7z", "docx", "xlsx", "pptx")
    private val NAME_FORMAT = DateTimeFormatter.ofPattern("yyyy-MM-dd HHmm")

    /** One thing in the copy: a file on the phone, or text made for it (the waiting notes). */
    class Entry(val name: String, val file: File?, val text: ByteArray?, val size: Long, val packed: Boolean, val modified: Long)

    /** What the copy holds. [bytes] is the files' size, the most the .zip can need. */
    class Plan(val entries: List<Entry>, val bytes: Long, val waiting: Int) {
        val files: Int get() = entries.size

        /** The database files come first, so HQ is held stopped only for them. */
        val databaseFiles: Int get() = entries.count { isDatabase(it.name) }
    }

    class Progress(val filesDone: Int, val filesTotal: Int, val bytesDone: Long, val bytesTotal: Long, val elapsedMs: Long)

    /** [leftOut] names the files that could not be read; [zipBytes] is the size of the .zip. */
    class Result(val files: Int, val zipBytes: Long, val leftOut: List<String>)

    class Cancelled : Exception()

    /** The file name offered to the picker, from the spec's pattern. */
    fun fileName(time: LocalDateTime): String = "HQ notes ${NAME_FORMAT.format(time)}.zip"

    /** True for what never goes in: logs, HQ's lock files, backups, restore work folders, the config and the bridge key. */
    fun excluded(relative: String, isDirectory: Boolean): Boolean {
        val name = relative.substringAfterLast('/')
        if (isDirectory) return name.startsWith(".hq-restore-") || name == ".hq" || name == "_logs"
        val lower = name.lowercase()
        return lower.endsWith(".log") || lower.endsWith(".lock") || lower.endsWith(".lck") || lower.endsWith(".pid") || lower.endsWith(".hqbak") ||
            name == "LOCK" || name.startsWith(".hq-restore-") || name == "bridge.key" || (relative == "config.yaml")
    }

    /**
     * What goes in the copy, read from [filesDir]: everything under the vault except [excluded], the database as `vault.db` with its
     * `-wal` and `-shm`, and `outbox.json` as `waiting-to-send.md` when something waits. With [withVault] false only the waiting
     * notes are planned (a phone whose notes are on a server HQ). [onCount] hears the file count as it grows.
     */
    fun plan(filesDir: File, withVault: Boolean = true, onCount: (Int) -> Unit = {}): Plan {
        val entries = ArrayList<Entry>()
        var bytes = 0L
        var waiting = 0
        if (withVault) {
            val root = File(filesDir, VAULT)
            val queue = ArrayDeque<Pair<File, String>>()
            if (root.isDirectory) queue.add(root to "")
            while (queue.isNotEmpty()) {
                val (dir, prefix) = queue.removeFirst()
                val children = dir.listFiles()?.sortedBy { it.name } ?: continue
                for (child in children) {
                    // A link could point at a file outside the vault, such as a key.
                    if (Files.isSymbolicLink(child.toPath())) continue
                    val relative = prefix + child.name
                    if (child.isDirectory) {
                        if (!excluded(relative, true)) queue.add(child to "$relative/")
                    } else if (child.isFile && !excluded(relative, false)) {
                        val size = child.length()
                        entries += Entry(relative, child, null, size, isPacked(child.name), child.lastModified())
                        bytes += size
                        if (entries.size % COUNT_EVERY == 0) onCount(entries.size)
                    }
                }
            }
        }
        val outbox = File(filesDir, OUTBOX)
        if (outbox.isFile) {
            val (text, count) = waitingText(outbox)
            if (text != null) {
                val data = text.toByteArray(Charsets.UTF_8)
                entries += Entry(WAITING_NAME, null, data, data.size.toLong(), false, outbox.lastModified())
                bytes += data.size
                waiting = count
            }
        }
        onCount(entries.size)
        return Plan(entries.sortedBy { if (isDatabase(it.name)) 0 else 1 }, bytes, waiting)
    }

    /** How many notes and tasks wait to send, without reading the vault. */
    fun waitingCount(filesDir: File): Int = waitingText(File(filesDir, OUTBOX)).second

    /** `waiting-to-send.md` for an outbox file, and how many items it holds; the text is null when nothing waits or the file cannot be read. */
    private fun waitingText(outbox: File): Pair<String?, Int> = try {
        val items = JSONObject(outbox.readText()).optJSONArray("items")
        val out = StringBuilder()
        var count = 0
        for (i in 0 until (items?.length() ?: 0)) {
            val item = items!!.optJSONObject(i) ?: continue
            val payload = item.optJSONObject("payload") ?: JSONObject()
            val title = item.optString("title").ifBlank { payload.optString("title") }.ifBlank { "Untitled" }
            if (count == 0) out.append("# Waiting to send\n\n")
            count++
            out.append("## ").append(title.replace('\n', ' ')).append(" (").append(item.optString("kind")).append(")\n\n")
            payload.optString("folder").takeIf { it.isNotBlank() }?.let { out.append("Folder: ").append(it).append("\n\n") }
            for (key in listOf("content", "description", "notes")) payload.optString(key).takeIf { it.isNotBlank() }?.let { out.append(it).append("\n\n") }
        }
        (if (count == 0) null else out.toString()) to count
    } catch (_: IOException) {
        null to 0
    } catch (_: JSONException) {
        null to 0
    }

    /** The database, its write-ahead log and its shared-memory file: they are only consistent together, so HQ is held stopped to copy them. */
    fun isDatabase(name: String): Boolean = name == "_data/vault.db" || name == "_data/vault.db-wal" || name == "_data/vault.db-shm"

    private fun isPacked(name: String): Boolean = name.substringAfterLast('.', "").lowercase() in PACKED

    /**
     * Streams the .zip into [out]: text compressed fast, pictures and PDFs as they are. [progress] hears at most every 200 ms and once at
     * the end; [cancelled] is asked between chunks and ends the copy with [Cancelled]. A file that cannot be read is left out and named in
     * `left-out.txt`; a failure of [out] is thrown as it came.
     */
    fun write(
        plan: Plan, out: java.io.OutputStream, now: () -> Long = System::currentTimeMillis, forceZip64: Boolean = false,
        cancelled: () -> Boolean = { false }, onDatabaseCopied: () -> Unit = {}, progress: (Progress) -> Unit = {},
    ): Result {
        val zip = ZipStream(out, forceZip64)
        val start = now()
        var last = start
        var bytes = 0L
        var files = 0
        val leftOut = ArrayList<String>()
        fun report(force: Boolean) {
            val t = now()
            if (force || t - last >= PROGRESS_EVERY_MS) { last = t; progress(Progress(files, plan.files, bytes, plan.bytes, t - start)) }
        }
        var databaseLeft = plan.databaseFiles
        if (databaseLeft == 0) onDatabaseCopied()
        for (entry in plan.entries) {
            if (cancelled()) throw Cancelled()
            val level = if (entry.packed) Deflater.NO_COMPRESSION else Deflater.BEST_SPEED
            val source = try {
                entry.text?.inputStream() ?: entry.file!!.inputStream()
            } catch (_: IOException) {
                leftOut += entry.name
                files++
                if (isDatabase(entry.name) && --databaseLeft == 0) onDatabaseCopied()
                continue
            }
            val complete = source.use { input ->
                zip.add(entry.name, entry.modified, level, entry.size, input) { n ->
                    if (cancelled()) throw Cancelled()
                    bytes += n
                    report(false)
                }
            }
            if (!complete) leftOut += entry.name
            files++
            if (isDatabase(entry.name) && --databaseLeft == 0) onDatabaseCopied()
            report(false)
        }
        if (leftOut.isNotEmpty()) {
            zip.add(LEFT_OUT_NAME, start, Deflater.BEST_SPEED, 0, leftOut.joinToString("\n", postfix = "\n").byteInputStream())
        }
        zip.close()
        report(true)
        return Result(files, zip.bytesWritten, leftOut)
    }

    /** True when the place cannot take the copy: the free space it reports is under what the copy needs. Null free space means unknown, so go on. */
    fun short(plan: Plan, free: Long?): Boolean = free != null && free < plan.bytes + SPACE_MARGIN

    /** How much to free up, in bytes, for a place that reported [free]. */
    fun freeUpBytes(plan: Plan, free: Long): Long = (plan.bytes + SPACE_MARGIN - free).coerceAtLeast(0)

    enum class Failure { PhoneFull, PlaceFull, PlaceGone }

    /** Which line a failed write gets: a full disk (ENOSPC) names the place, anything else says the place stopped answering. */
    fun classify(e: Throwable): Failure {
        var t: Throwable? = e
        while (t != null) {
            val m = t.message.orEmpty()
            if (m.contains("ENOSPC") || m.contains("No space left", true) || m.contains("disk full", true)) return Failure.PlaceFull
            t = t.cause
        }
        return Failure.PlaceGone
    }

    /** Seconds left at the pace so far, or null before there is a pace to go by. */
    fun secondsLeft(p: Progress): Long? {
        if (p.bytesDone <= 0 || p.elapsedMs < 1_000) return null
        val perMs = p.bytesDone.toDouble() / p.elapsedMs
        return ((p.bytesTotal - p.bytesDone) / perMs / 1_000).toLong().coerceAtLeast(0)
    }

    /** A size as a person reads it: 212 kB, 340 MB, 1.2 GB. */
    fun size(bytes: Long): String = when {
        bytes < 1_000 -> "$bytes B"
        bytes < 1_000_000 -> "${bytes / 1_000} kB"
        bytes < 1_000_000_000 -> "${bytes / 1_000_000} MB"
        else -> "%.1f GB".format(java.util.Locale.ROOT, bytes / 1e9)
    }

    /** The copy that was started and not finished, recorded before writing so a killed process can clean up on the next open. */
    fun markPending(filesDir: File, documentUri: String) { File(filesDir, PENDING).writeText(documentUri) }
    fun pending(filesDir: File): String? = File(filesDir, PENDING).takeIf { it.isFile }?.readText()?.takeIf { it.isNotBlank() }
    fun clearPending(filesDir: File) { File(filesDir, PENDING).delete() }

    /** When the last copy finished, for Backup and the copy-first rule. */
    fun recordCopy(filesDir: File, atMs: Long) { File(filesDir, LAST_COPY).writeText(atMs.toString()) }
    fun lastCopy(filesDir: File): Long? = File(filesDir, LAST_COPY).takeIf { it.isFile }?.readText()?.trim()?.toLongOrNull()

    private const val OFF_PHONE = "backup-off-phone"

    /** Whether a document lives in this phone's own storage (internal storage, Downloads, media), where losing the phone loses the copy. */
    fun onThisPhone(authority: String?, documentId: String?): Boolean = when (authority) {
        "com.android.externalstorage.documents" -> documentId?.startsWith("primary") == true
        "com.android.providers.downloads.documents", "com.android.providers.media.documents" -> true
        else -> false
    }

    /** Records a finished copy: always as the last copy, and as the last copy off this phone only when it was saved somewhere else. */
    fun recordCopy(filesDir: File, atMs: Long, onPhone: Boolean) {
        recordCopy(filesDir, atMs)
        if (!onPhone) File(filesDir, OFF_PHONE).writeText(atMs.toString())
    }

    fun lastCopyOffPhone(filesDir: File): Long? = File(filesDir, OFF_PHONE).takeIf { it.isFile }?.readText()?.trim()?.toLongOrNull()
}
