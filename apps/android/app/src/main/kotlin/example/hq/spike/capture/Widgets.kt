package example.hq.spike.capture

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.os.SystemClock
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.style.ForegroundColorSpan
import android.text.style.StyleSpan
import android.view.View
import android.widget.RemoteViews
import example.hq.spike.AlertPoster
import example.hq.spike.R
import example.hq.spike.data.HqServices
import java.time.LocalTime
import java.time.format.DateTimeFormatter
import kotlinx.coroutines.runBlocking
import org.json.JSONObject

/** The three home screen widgets, drawn from what HQ last said. They never fetch on their own: the hub's poll pushes into them. */
class CaptureWidget : AppWidgetProvider() {
    override fun onUpdate(c: Context, m: AppWidgetManager, ids: IntArray) = Widgets.renderAll(c)
}

class NeedsYouWidget : AppWidgetProvider() {
    override fun onUpdate(c: Context, m: AppWidgetManager, ids: IntArray) = Widgets.renderAll(c)
}

class TasksWidget : AppWidgetProvider() {
    override fun onUpdate(c: Context, m: AppWidgetManager, ids: IntArray) = Widgets.renderAll(c)
}

/** What the Needs you widget says: a count, or that it is quiet and when that was last checked. */
data class NeedsFacts(val count: Int, val title: String?, val checkedAt: String, val checkedMs: Long = 0L)

/** One row of the Tasks widget: the task's id for opening it, its number and its title. */
data class TaskLine(val id: String, val number: String, val title: String)

object Widgets {
    private const val FILE = "widgets"
    private const val COUNT = "needs_count"
    private const val TITLE = "needs_title"
    private const val CHECKED = "checked"
    private const val CHECKED_MS = "checked_ms"
    /** How old the last answer from HQ may be before the widget says when it was read. */
    private const val STALE_MS = 10 * 60 * 1000L
    /** The most items one poll reads, so a count at this size means "at least". */
    private const val NEEDS_CAP = 30
    private const val TASKS = "tasks"
    private const val TASKS_TOTAL = "tasks_total"
    private const val TASKS_EVERY_MS = 60_000L
    private const val MAX_ROWS = 3
    private val CLOCK = DateTimeFormatter.ofPattern("HH:mm")

    @Volatile private var tasksAt = 0L

    private fun prefs(c: Context) = c.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    /** The last thing HQ said about what needs the owner, kept so the widget has something to show while HQ is still starting. */
    fun needsFacts(c: Context): NeedsFacts = prefs(c).let { NeedsFacts(it.getInt(COUNT, 0), it.getString(TITLE, null), it.getString(CHECKED, "") ?: "", it.getLong(CHECKED_MS, 0L)) }

    /** Called by the hub's poll with how many items wait and the newest one's title. */
    @JvmStatic
    fun needs(c: Context, count: Int, newestTitle: String?) {
        prefs(c).edit().putInt(COUNT, count).putString(TITLE, newestTitle).putString(CHECKED, LocalTime.now().format(CLOCK)).putLong(CHECKED_MS, System.currentTimeMillis()).apply()
        renderAll(c)
    }

    /** Called by the hub's poll: reads the tasks in progress at most once a minute, and only when such a widget is on a screen. */
    @JvmStatic
    fun refreshTasks(c: Context) {
        val now = SystemClock.elapsedRealtime()
        if (now - tasksAt < TASKS_EVERY_MS || !present(c, TasksWidget::class.java)) return
        tasksAt = now
        val client = HqServices.get(c.applicationContext).background
        // HQ may be starting or down: the widget keeps what it last showed.
        val rows = try { runBlocking { client.inProgress(MAX_ROWS) } } catch (e: example.hq.spike.data.HqUnavailable) { return }
        prefs(c).edit().putString(TASKS, rows.first.joinToString("\n") { "${it.id}\t${it.number}\t${it.title.replace('\n', ' ')}" }).putInt(TASKS_TOTAL, rows.second).apply()
        renderAll(c)
    }

    fun taskLines(c: Context): Pair<List<TaskLine>, Int> {
        val p = prefs(c)
        val lines = p.getString(TASKS, "").orEmpty().lines().filter { it.isNotBlank() }.mapNotNull { l -> l.split('\t', limit = 3).takeIf { it.size == 3 }?.let { TaskLine(it[0], it[1], it[2]) } }
        return lines to p.getInt(TASKS_TOTAL, lines.size)
    }

    fun present(c: Context, kind: Class<*>): Boolean = AppWidgetManager.getInstance(c).getAppWidgetIds(ComponentName(c, kind)).isNotEmpty()

    fun renderAll(c: Context) {
        val m = AppWidgetManager.getInstance(c)
        for (id in m.getAppWidgetIds(ComponentName(c, CaptureWidget::class.java))) m.updateAppWidget(id, capture(c))
        for (id in m.getAppWidgetIds(ComponentName(c, NeedsYouWidget::class.java))) m.updateAppWidget(id, needsView(c))
        for (id in m.getAppWidgetIds(ComponentName(c, TasksWidget::class.java))) m.updateAppWidget(id, tasksView(c))
    }

    private fun captureIntent(c: Context, code: Int, mode: CaptureMode?): PendingIntent {
        val i = Intent(c, CaptureActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        mode?.let { i.putExtra(CaptureActivity.EXTRA_MODE, it.name.lowercase()) }
        return PendingIntent.getActivity(c, code, i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    private fun capture(c: Context) = RemoteViews(c.packageName, R.layout.widget_capture).apply {
        setOnClickPendingIntent(R.id.capture_field, captureIntent(c, 1, CaptureMode.Note))
        setOnClickPendingIntent(R.id.capture_text, captureIntent(c, 1, CaptureMode.Note))
        setOnClickPendingIntent(R.id.capture_note, captureIntent(c, 2, CaptureMode.Note))
        setOnClickPendingIntent(R.id.capture_task, captureIntent(c, 3, CaptureMode.Task))
    }

    fun needsView(c: Context): RemoteViews {
        val f = needsFacts(c)
        val showTitles = CapturePrefs.showTitles(c)
        return RemoteViews(c.packageName, R.layout.widget_needs).apply {
            setOnClickPendingIntent(R.id.needs_root, AlertPoster.open(c, "hq://inbox"))
            val stale = f.checkedMs != 0L && System.currentTimeMillis() - f.checkedMs > STALE_MS
            val footer = if (stale && f.checkedAt.isNotBlank()) "Last checked ${f.checkedAt}" else ""
            when {
                f.checkedMs == 0L -> {
                    setTextViewText(R.id.needs_label, "HQ")
                    setViewVisibility(R.id.needs_count, View.GONE)
                    setTextViewText(R.id.needs_hint, "Waiting for HQ")
                    setTextViewText(R.id.needs_footer, "")
                }
                f.count > 0 -> {
                    setTextViewText(R.id.needs_label, "NEEDS YOU")
                    setViewVisibility(R.id.needs_count, View.VISIBLE)
                    setTextViewText(R.id.needs_count, if (f.count >= NEEDS_CAP) "$NEEDS_CAP+" else f.count.toString())
                    setTextViewText(R.id.needs_hint, if (showTitles && !f.title.isNullOrBlank()) f.title else "Tap to review")
                    setTextViewText(R.id.needs_footer, footer)
                }
                else -> {
                    setTextViewText(R.id.needs_label, "QUIET")
                    setViewVisibility(R.id.needs_count, View.GONE)
                    setTextViewText(R.id.needs_hint, "Nothing needs you right now.")
                    setTextViewText(R.id.needs_footer, if (f.checkedAt.isBlank()) "" else if (stale) footer else "Checked ${f.checkedAt}")
                }
            }
        }
    }

    fun tasksView(c: Context): RemoteViews {
        val (lines, total) = taskLines(c)
        val rows = intArrayOf(R.id.task_row1, R.id.task_row2, R.id.task_row3)
        return RemoteViews(c.packageName, R.layout.widget_tasks).apply {
            setTextViewText(R.id.tasks_label, "IN PROGRESS · $total")
            setOnClickPendingIntent(R.id.tasks_add, captureIntent(c, 4, CaptureMode.Task))
            setViewVisibility(R.id.tasks_empty, if (lines.isEmpty()) View.VISIBLE else View.GONE)
            rows.forEachIndexed { i, rowId ->
                val line = lines.getOrNull(i)
                setViewVisibility(rowId, if (line == null) View.GONE else View.VISIBLE)
                if (line != null) {
                    setTextViewText(rowId, numbered(line))
                    setOnClickPendingIntent(rowId, AlertPoster.open(c, "hq://tasks/${line.id}"))
                }
            }
        }
    }

    /** "061 Workstream C": the number in white and bold, then the title. */
    private fun numbered(line: TaskLine): CharSequence = SpannableStringBuilder().apply {
        append(line.number)
        setSpan(StyleSpan(android.graphics.Typeface.BOLD), 0, length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        setSpan(ForegroundColorSpan(Color.WHITE), 0, length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        append(' ').append(line.title)
    }

    /** The number is the last part of the display id ("PERSONAL-INBOX-061" is "061"). */
    fun numberOf(displayId: String): String = displayId.substringAfterLast('-').ifBlank { displayId }

    /** Asks Android to put a widget on the home screen; false when the launcher cannot. */
    fun pin(c: Context, kind: Class<*>): Boolean {
        val m = AppWidgetManager.getInstance(c)
        return m.isRequestPinAppWidgetSupported && m.requestPinAppWidget(ComponentName(c, kind), null, null)
    }

    fun json(o: JSONObject): String = o.toString()
}
