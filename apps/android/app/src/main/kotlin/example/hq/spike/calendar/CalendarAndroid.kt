package example.hq.spike.calendar

import android.Manifest
import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.content.pm.PackageManager
import android.provider.CalendarContract
import android.provider.CalendarContract.Calendars
import android.provider.CalendarContract.Events
import android.provider.CalendarContract.Instances

/** The real Calendar Provider. Insert and read go through the content resolver on whatever thread calls; callers use a background one. */
class AndroidCalendarStore(context: Context) : CalendarStore {
    private val app = context.applicationContext

    override fun hasPermission(): Boolean = hasPermission(app)

    override fun writableCalendars(): List<WritableCalendar> {
        if (!hasPermission()) return emptyList()
        val out = mutableListOf<WritableCalendar>()
        try {
            app.contentResolver.query(
                Calendars.CONTENT_URI,
                arrayOf(Calendars._ID, Calendars.CALENDAR_DISPLAY_NAME, Calendars.ACCOUNT_NAME, Calendars.ACCOUNT_TYPE),
                "${Calendars.CALENDAR_ACCESS_LEVEL} >= ? AND ${Calendars.VISIBLE} = 1",
                arrayOf(Calendars.CAL_ACCESS_CONTRIBUTOR.toString()),
                Calendars.CALENDAR_DISPLAY_NAME,
            )?.use { c ->
                while (c.moveToNext()) {
                    val local = c.getString(COL_TYPE) == CalendarContract.ACCOUNT_TYPE_LOCAL
                    out += WritableCalendar(c.getLong(COL_ID), c.getString(COL_NAME).orEmpty(), if (local) LOCAL_LABEL else c.getString(COL_ACCOUNT).orEmpty())
                }
            }
        } catch (e: SecurityException) {
            return emptyList()
        } catch (e: IllegalArgumentException) {
            return emptyList()
        }
        return out
    }

    override fun insert(calendarId: Long, request: CalendarRequest): Long? {
        val values = ContentValues().apply {
            put(Events.CALENDAR_ID, calendarId)
            put(Events.TITLE, request.title)
            put(Events.DTSTART, request.span.startMs)
            put(Events.DTEND, request.span.endMs)
            put(Events.EVENT_TIMEZONE, request.span.timeZone)
            put(Events.ALL_DAY, if (request.allDay) 1 else 0)
            request.location?.let { put(Events.EVENT_LOCATION, it) }
            request.description?.let { put(Events.DESCRIPTION, it) }
        }
        val uri = try {
            app.contentResolver.insert(Events.CONTENT_URI, values)
        } catch (e: SecurityException) {
            throw CalendarStoreException("insert not allowed", e)
        } catch (e: IllegalArgumentException) {
            throw CalendarStoreException("insert rejected", e)
        }
        return uri?.let { ContentUris.parseId(it) }
    }

    override fun read(eventId: Long): StoredEvent? = try {
        app.contentResolver.query(
            ContentUris.withAppendedId(Events.CONTENT_URI, eventId),
            arrayOf(Events.CALENDAR_ID, Events.TITLE, Events.DTSTART, Events.DTEND, Events.EVENT_TIMEZONE, Events.ALL_DAY, Events.DELETED),
            null, null, null,
        )?.use { c ->
            if (!c.moveToFirst() || c.getInt(COL_DELETED) == 1) null
            else StoredEvent(c.getLong(0), c.getString(1).orEmpty(), c.getLong(2), c.getLong(3), c.getString(4).orEmpty(), c.getInt(5) == 1)
        }
    } catch (e: SecurityException) {
        throw CalendarStoreException("read not allowed", e)
    } catch (e: IllegalArgumentException) {
        throw CalendarStoreException("read rejected", e)
    }

    companion object {
        private const val COL_ID = 0
        private const val COL_NAME = 1
        private const val COL_ACCOUNT = 2
        private const val COL_TYPE = 3
        private const val COL_DELETED = 6
        private const val LOCAL_LABEL = "On this phone"

        fun hasPermission(c: Context): Boolean =
            c.checkSelfPermission(Manifest.permission.READ_CALENDAR) == PackageManager.PERMISSION_GRANTED &&
                c.checkSelfPermission(Manifest.permission.WRITE_CALENDAR) == PackageManager.PERMISSION_GRANTED
    }
}

/** Instances for the list action. Only the five columns the contract allows are queried; nothing else about an event is read. */
class AndroidCalendarReader(context: Context) : CalendarReader {
    private val app = context.applicationContext

    override fun hasPermission(): Boolean = app.checkSelfPermission(Manifest.permission.READ_CALENDAR) == PackageManager.PERMISSION_GRANTED

    override fun instances(fromMs: Long, toMs: Long): List<CalendarInstance> {
        val uri = CalendarContract.Instances.CONTENT_URI.buildUpon().also {
            ContentUris.appendId(it, fromMs)
            ContentUris.appendId(it, toMs)
        }.build()
        val out = mutableListOf<CalendarInstance>()
        try {
            app.contentResolver.query(
                uri,
                arrayOf(Instances.EVENT_ID, Instances.TITLE, Instances.BEGIN, Instances.END, Instances.ALL_DAY, Instances.CALENDAR_DISPLAY_NAME, Instances.VISIBLE),
                "${Instances.VISIBLE} = 1 AND ${Events.DELETED} = 0",
                null,
                "${Instances.BEGIN} ASC, ${Instances.EVENT_ID} ASC",
            )?.use { c ->
                while (c.moveToNext()) {
                    out += CalendarInstance(c.getLong(0), c.getString(1).orEmpty(), c.getLong(2), c.getLong(3), c.getInt(4) == 1, c.getString(5).orEmpty(), c.getInt(6) == 1)
                }
            }
        } catch (e: SecurityException) {
            throw CalendarStoreException("read not allowed", e)
        } catch (e: IllegalArgumentException) {
            throw CalendarStoreException("read rejected", e)
        }
        return out
    }
}

/** Edit and delete through the content resolver. Only the columns the edit checks need are read; callers use a background thread. */
class AndroidCalendarEditor(context: Context) : CalendarEditor {
    private val app = context.applicationContext

    override fun hasPermission(): Boolean = AndroidCalendarStore.hasPermission(app)

    override fun row(eventId: Long): EventRow? = guarded("read") {
        app.contentResolver.query(
            ContentUris.withAppendedId(Events.CONTENT_URI, eventId),
            arrayOf(
                Events.CALENDAR_ID, Events.TITLE, Events.DTSTART, Events.DTEND, Events.ALL_DAY, Events.RRULE, Events.RDATE,
                Events.ORIGINAL_ID, Events.ORIGINAL_INSTANCE_TIME, Events.DELETED,
            ),
            null, null, null,
        )?.use { c ->
            if (!c.moveToFirst()) return@use null
            val start = c.getLong(COL_START)
            EventRow(
                calendarId = c.getLong(COL_CALENDAR),
                title = c.getString(COL_TITLE).orEmpty(),
                startMs = start,
                endMs = if (c.isNull(COL_END)) start else c.getLong(COL_END),
                allDay = c.getInt(COL_ALL_DAY) == 1,
                rrule = c.getString(COL_RRULE).orEmpty(),
                rdate = c.getString(COL_RDATE).orEmpty(),
                originalId = c.getString(COL_ORIGINAL_ID).orEmpty(),
                originalInstanceTime = if (c.isNull(COL_ORIGINAL_TIME)) null else c.getLong(COL_ORIGINAL_TIME),
                deleted = c.getInt(COL_DELETED) == 1,
                accessLevel = accessLevel(c.getLong(COL_CALENDAR)),
            )
        }
    }

    /** Zero when the calendar is gone, which the caller treats as read-only. */
    private fun accessLevel(calendarId: Long): Int = app.contentResolver.query(
        ContentUris.withAppendedId(Calendars.CONTENT_URI, calendarId),
        arrayOf(Calendars.CALENDAR_ACCESS_LEVEL), null, null, null,
    )?.use { if (it.moveToFirst()) it.getInt(0) else 0 } ?: 0

    override fun update(eventId: Long, write: EventWrite): Boolean = guarded("update") {
        val values = ContentValues().apply {
            write.title?.let { put(Events.TITLE, it) }
            write.startMs?.let { put(Events.DTSTART, it) }
            write.endMs?.let { put(Events.DTEND, it) }
        }
        app.contentResolver.update(ContentUris.withAppendedId(Events.CONTENT_URI, eventId), values, null, null) > 0
    }

    override fun delete(eventId: Long): Boolean = guarded("delete") {
        app.contentResolver.delete(ContentUris.withAppendedId(Events.CONTENT_URI, eventId), null, null) > 0
    }

    private fun <T> guarded(what: String, block: () -> T): T = try {
        block()
    } catch (e: SecurityException) {
        throw CalendarStoreException("$what not allowed", e)
    } catch (e: IllegalArgumentException) {
        throw CalendarStoreException("$what rejected", e)
    }

    private companion object {
        const val COL_CALENDAR = 0
        const val COL_TITLE = 1
        const val COL_START = 2
        const val COL_END = 3
        const val COL_ALL_DAY = 4
        const val COL_RRULE = 5
        const val COL_RDATE = 6
        const val COL_ORIGINAL_ID = 7
        const val COL_ORIGINAL_TIME = 8
        const val COL_DELETED = 9
    }
}

/** Small private stores: the ledger and the preferred calendar, in app-private preferences like the app's other small stores. */
class CalendarPrefs(context: Context) : CalendarLedger, PreferredCalendar {
    private val prefs = context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    override fun get(requestId: String): Long? = if (prefs.contains(LEDGER + requestId)) prefs.getLong(LEDGER + requestId, 0L) else null

    override fun put(requestId: String, eventId: Long) {
        trimLedger()
        // commit, not apply: the entry must survive a process death right after the insert, or a retry could insert twice.
        prefs.edit().putLong(LEDGER + requestId, eventId).putLong(STAMP + requestId, System.currentTimeMillis()).commit()
    }

    override fun get(): Long? = if (prefs.contains(PREFERRED)) prefs.getLong(PREFERRED, 0L) else null

    fun setPreferred(id: Long) { prefs.edit().putLong(PREFERRED, id).apply() }

    /** Keeps the ledger from growing for ever: the oldest entries go once there are more than [LEDGER_MAX]. */
    private fun trimLedger() {
        val stamps = prefs.all.filterKeys { it.startsWith(STAMP) }.mapValues { (it.value as? Long) ?: 0L }
        if (stamps.size < LEDGER_MAX) return
        val edit = prefs.edit()
        stamps.entries.sortedBy { it.value }.take(stamps.size - LEDGER_MAX + 1).forEach { edit.remove(it.key).remove(LEDGER + it.key.removePrefix(STAMP)) }
        edit.apply()
    }

    private companion object {
        const val FILE = "calendar_actions"
        const val PREFERRED = "preferred_calendar"
        const val LEDGER = "ledger_"
        const val STAMP = "stamp_"
        const val LEDGER_MAX = 200
    }
}
