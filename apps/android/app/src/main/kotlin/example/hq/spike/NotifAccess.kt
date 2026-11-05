package example.hq.spike

import android.Manifest
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.compose.runtime.staticCompositionLocalOf
import example.hq.spike.measure.Counter
import example.hq.spike.measure.Counters

/** What the phone says about notifications right now, plus the two things only this app can remember. */
data class NotifFactsNow(
    val sdk: Int,
    val granted: Boolean,
    val appEnabled: Boolean,
    /** The Needs you channel's importance, or [NotificationManager.IMPORTANCE_DEFAULT] or more when it is not created yet. */
    val needsImportance: Int,
    val rationale: Boolean,
    val asked: Boolean,
) {
    /** True when notifications reach the owner at all and Needs you is not blocked. */
    val needsOn: Boolean get() = (sdk < 33 || granted) && appEnabled && needsImportance != NotificationManager.IMPORTANCE_NONE
}

/** The one step that gets the owner from here to "Needs you can tell me". */
enum class NotifStep { Done, AskAndroid, OpenApp, OpenChannel }

/** Where a Turn on came from, so the right counter moves. */
enum class NotifOrigin { FirstRun, Later }

/** What the screens read and call. Provided by the activity. */
class NotifUi(val facts: NotifFactsNow, val turnOn: (NotifOrigin) -> Unit, val justTurnedOn: Boolean = false)

val LocalNotif = staticCompositionLocalOf<NotifUi?> { null }

/** Notification access: the pure table of next steps and the small store behind it. */
object NotifAccess {
    private const val FILE = "notif_access"
    private const val ASKED = "asked"
    private const val RATIONALE = "rationale"
    private const val LAST_VERSION = "last_version"
    private const val UPGRADER_SHOWN = "upgrader_shown"
    private const val SNOOZE_UNTIL = "banner_snooze_until"
    const val WAITING = "waiting"
    const val SERVER_WAITING = "server_waiting"
    private const val ALERTS = "alerts"
    private const val SEP = "\n"
    const val SNOOZE_MS = 7L * 24 * 60 * 60 * 1000

    /** The spec's table, in order. Quieter-only Needs you is Done: Settings carries a note, nothing here asks. */
    fun next(f: NotifFactsNow): NotifStep = when {
        f.sdk >= 33 && !f.granted -> if (!f.asked || f.rationale) NotifStep.AskAndroid else NotifStep.OpenApp
        !f.appEnabled -> NotifStep.OpenApp
        f.needsImportance == NotificationManager.IMPORTANCE_NONE -> NotifStep.OpenChannel
        else -> NotifStep.Done
    }

    /** The upgrader sheet shows on Android 13+ once, on the first open after an update, to someone never asked and not allowed. */
    fun showUpgrader(sdk: Int, storedVersion: Int, currentVersion: Int, granted: Boolean, asked: Boolean, shownBefore: Boolean): Boolean =
        sdk >= 33 && storedVersion in 1 until currentVersion && !granted && !asked && !shownBefore

    /** The ids kept as one newline-joined string, as the poll loop stores them. */
    @JvmStatic
    fun ids(stored: String?): MutableList<String> = stored.orEmpty().split(SEP).filter { it.isNotEmpty() }.toMutableList()

    /** How many questions waited while Needs you could not reach the owner: this phone's HQ first, the server HQ's apart. */
    fun waited(c: Context): Pair<Int, Int> {
        val p = c.applicationContext.getSharedPreferences(ALERTS, Context.MODE_PRIVATE)
        return ids(p.getString(WAITING, "")).size to ids(p.getString(SERVER_WAITING, "")).size
    }

    fun bannerVisible(f: NotifFactsNow, snoozeUntil: Long, now: Long): Boolean = !f.needsOn && now >= snoozeUntil

    /** The Inbox banner's two counts, or null while it should not show. */
    fun banner(f: NotifFactsNow, snoozeUntil: Long, now: Long, waited: Pair<Int, Int>): example.hq.spike.ui.inbox.InboxBanner? =
        if (bannerVisible(f, snoozeUntil, now)) example.hq.spike.ui.inbox.InboxBanner(waited.first, waited.second) else null

    private fun prefs(c: Context) = c.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    fun read(c: Context): NotifFactsNow {
        val nm = c.getSystemService(NotificationManager::class.java)
        val granted = Build.VERSION.SDK_INT < 33 || c.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
        val needs = if (Build.VERSION.SDK_INT >= 26) nm?.getNotificationChannel(AlertPoster.NEEDS)?.importance ?: NotificationManager.IMPORTANCE_HIGH else NotificationManager.IMPORTANCE_HIGH
        val p = prefs(c)
        return NotifFactsNow(Build.VERSION.SDK_INT, granted, nm?.areNotificationsEnabled() ?: true, needs, p.getBoolean(RATIONALE, false), p.getBoolean(ASKED, false))
    }

    /** Called with the dialog's answer. [rationale] is read by the activity, since only an activity can. */
    fun answered(c: Context, origin: NotifOrigin, granted: Boolean, rationale: Boolean) {
        prefs(c).edit().putBoolean(ASKED, true).putBoolean(RATIONALE, rationale).apply()
        val counter = when {
            !granted -> Counter.NotificationsNotAllowed
            origin == NotifOrigin.FirstRun -> Counter.NotificationsAllowedFirstRun
            else -> Counter.NotificationsAllowedLater
        }
        Counters.get(c).bump(counter)
    }

    /** Counts a grant made in Android settings after a Turn on that sent the owner there. */
    fun grantedInSettings(c: Context, origin: NotifOrigin) {
        Counters.get(c).bump(if (origin == NotifOrigin.FirstRun) Counter.NotificationsAllowedFirstRun else Counter.NotificationsAllowedLater)
    }

    fun storedVersion(c: Context): Int = prefs(c).getInt(LAST_VERSION, 0)
    fun saveVersion(c: Context, version: Int) { prefs(c).edit().putInt(LAST_VERSION, version).apply() }
    fun upgraderShown(c: Context): Boolean = prefs(c).getBoolean(UPGRADER_SHOWN, false)
    fun markUpgraderShown(c: Context) { prefs(c).edit().putBoolean(UPGRADER_SHOWN, true).apply() }

    fun snoozeUntil(c: Context): Long = prefs(c).getLong(SNOOZE_UNTIL, 0L)
    fun snooze(c: Context, now: Long = System.currentTimeMillis()) { prefs(c).edit().putLong(SNOOZE_UNTIL, now + SNOOZE_MS).apply() }
    fun clearSnooze(c: Context) { prefs(c).edit().remove(SNOOZE_UNTIL).apply() }
}
