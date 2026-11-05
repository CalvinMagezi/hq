package example.hq.spike.measure

import android.content.Context
import android.net.ConnectivityManager
import java.io.File
import java.io.IOException
import java.time.LocalDate
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/**
 * The fixed list of things HQ counts on this phone ("What HQ noticed"). Each is a whole number or a small group of them
 * ([slots]); no id, title, name, path or time is ever kept. This package imports nothing that can reach a network.
 */
enum class Counter(val key: String, val slots: Int = 1) {
    NotificationsAllowedFirstRun("notifications.allowed_first_run"),
    NotificationsAllowedLater("notifications.allowed_later"),
    NotificationsNotAllowed("notifications.not_allowed"),

    /** 1 while HQ is the phone's assistant, else 0: set, not bumped. */
    AssistantChosen("assistant.chosen"),
    LockedUnlocked("locked_press.unlocked"),
    LockedCancelled("locked_press.cancelled"),
    NoticePosted("answer_notice.posted"),
    NoticeTapped("answer_notice.tapped"),

    /** Slot 0 tapped, slot 1 posted. */
    NoticeServerTapped("answer_notice.server_tapped", 2),

    /** Bytes: slot 0 on mobile data, slot 1 on Wi-Fi. Shown in MB. */
    DataAppUpdates("data_mb.app_updates", 2),
    DataSpeechModel("data_mb.speech_model", 2),
    DataServerChecks("data_mb.server_checks", 2),
    DataOther("data_mb.other", 2),

    /** Slot 0 Undo tapped, slot 1 actions shown with Undo. */
    UndoTaskDiscarded("undo.task_discarded", 2),
    UndoNoteDeleted("undo.note_deleted", 2),
    UndoHelperStopped("undo.helper_stopped", 2),
    StartWhileRunning("start_hq.while_running"),
    StartClosedWhileOpening("start.closed_while_opening"),
    SafeCopySaved("safe_copy.saved"),

    /** Slot 0 under an hour, slot 1 up to a day, slot 2 older. */
    SavedCopyAge("saved_copy.age", 3),
}

/** Where a download's bytes are counted. */
enum class DataPurpose(val counter: Counter) {
    AppUpdates(Counter.DataAppUpdates),
    SpeechModel(Counter.DataSpeechModel),
    ServerChecks(Counter.DataServerChecks),
    Other(Counter.DataOther),
}

/** One version's numbers: [version] since [since] (an ISO date). */
data class CounterColumn(val version: String, val since: String, val values: Map<Counter, List<Long>>) {
    fun slot(counter: Counter, slot: Int = 0): Long = values[counter]?.getOrNull(slot) ?: 0L
}

/** Everything the page and Copy details read: the switch, the day counting stopped, this version and the one before. */
data class CounterSnapshot(val on: Boolean, val stoppedOn: String, val current: CounterColumn, val previous: CounterColumn?)

/**
 * The counters file, `noBackupFilesDir/counters.json`, so no backup or phone-to-phone copy takes it. Writes go to a
 * temporary file renamed into place; a file that cannot be read starts again at zero. Two columns are kept: this version and
 * the one before it.
 */
class Counters internal constructor(private val file: File, private val version: String, private val today: () -> String) {
    private var on = true
    private var stoppedOn = ""
    private var current = CounterColumn(version, today(), emptyMap())
    private var previous: CounterColumn? = null

    init {
        load()
        if (current.version != version) {
            previous = current.takeIf { it.version.isNotBlank() }
            current = CounterColumn(version, today(), emptyMap())
            save()
        }
    }

    /** Adds [by] to one slot of [counter]. Does nothing while counting is off. */
    @Synchronized
    fun bump(counter: Counter, slot: Int = 0, by: Long = 1) {
        if (!on || slot !in 0 until counter.slots || by <= 0) return
        change(counter) { it[slot] = minOf(MAX_VALUE, it[slot] + by) }
    }

    /** Bytes the app itself read for [purpose], on mobile data or not. */
    fun addBytes(purpose: DataPurpose, metered: Boolean, n: Long) = bump(purpose.counter, if (metered) 0 else 1, n)

    /** Records a state, such as "HQ is the assistant" (1) or not (0). */
    @Synchronized
    fun set(counter: Counter, value: Long, slot: Int = 0) {
        if (!on || slot !in 0 until counter.slots) return
        change(counter) { it[slot] = value.coerceIn(0, MAX_VALUE) }
    }

    @Synchronized
    fun snapshot() = CounterSnapshot(on, stoppedOn, current, previous)

    /** The switch: off stops every counter at once and keeps the numbers. */
    @Synchronized
    fun setOn(value: Boolean) {
        if (value == on) return
        on = value
        stoppedOn = if (value) "" else today()
        save()
    }

    /** Both columns back to zero; counting carries on from now. */
    @Synchronized
    fun clear() {
        current = CounterColumn(version, today(), emptyMap())
        previous = null
        save()
    }

    private fun change(counter: Counter, edit: (LongArray) -> Unit) {
        val slots = LongArray(counter.slots) { current.slot(counter, it) }
        edit(slots)
        current = current.copy(values = current.values + (counter to slots.toList()))
        save()
    }

    private fun load() {
        if (!file.isFile || file.length() > MAX_BYTES) return
        try {
            val root = JSONObject(file.readText())
            on = root.optBoolean("on", true)
            stoppedOn = root.optString("stopped", "")
            current = column(root.optJSONObject("current")) ?: current
            previous = column(root.optJSONObject("previous"))
        } catch (_: JSONException) {
            on = true
            stoppedOn = ""
            previous = null
        } catch (_: IOException) {
            previous = null
        }
    }

    private fun column(o: JSONObject?): CounterColumn? {
        o ?: return null
        val counts = o.optJSONObject("c") ?: JSONObject()
        val values = buildMap {
            for (counter in Counter.entries) {
                val arr = counts.optJSONArray(counter.key) ?: continue
                put(counter, List(counter.slots) { i -> whole(arr.opt(i)) })
            }
        }
        return CounterColumn(o.optString("v", ""), o.optString("d", ""), values)
    }

    private fun whole(v: Any?): Long = if (v is Int || v is Long) (v as Number).toLong().coerceIn(0, MAX_VALUE) else 0L

    private fun encode(c: CounterColumn) = JSONObject().put("v", c.version).put("d", c.since).put(
        "c",
        JSONObject().also { counts -> c.values.forEach { (k, v) -> counts.put(k.key, JSONArray(v)) } },
    )

    private fun save() {
        try {
            val root = JSONObject().put("on", on).put("stopped", stoppedOn).put("current", encode(current))
            previous?.let { root.put("previous", encode(it)) }
            val text = root.toString()
            if (text.toByteArray().size > MAX_BYTES) return
            file.parentFile?.mkdirs()
            val tmp = File(file.path + ".tmp")
            tmp.writeText(text)
            if (!tmp.renameTo(file)) tmp.delete()
        } catch (_: IOException) {
            // Counting must never break what it counts.
        }
    }

    companion object {
        const val FILE = "counters.json"
        const val MAX_BYTES = 8 * 1024
        private const val MAX_VALUE = 999_999_999_999L
        @Volatile private var shared: Counters? = null

        /** The one set of counters for this app. */
        @JvmStatic
        fun get(context: Context): Counters = shared ?: synchronized(this) {
            shared ?: Counters(File(context.noBackupFilesDir, FILE), versionName(context)) { LocalDate.now().toString() }.also { shared = it }
        }

        /** True on mobile data or any network the owner may pay for by the megabyte. */
        @JvmStatic
        fun metered(context: Context): Boolean = context.getSystemService(ConnectivityManager::class.java)?.isActiveNetworkMetered ?: false

        private fun versionName(context: Context): String =
            context.packageManager.getPackageInfo(context.packageName, 0).versionName.orEmpty().ifBlank { "dev" }
    }
}
