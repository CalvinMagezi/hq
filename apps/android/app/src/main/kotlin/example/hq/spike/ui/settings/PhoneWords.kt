package example.hq.spike.ui.settings

import android.app.ApplicationExitInfo
import example.hq.spike.AlertPoster
import example.hq.spike.R
import java.time.Instant

/** What Android says about how it treats this app, read on the phone. A null means this Android version has no such setting. */
data class KeepFacts(
    val batteryUnrestricted: Boolean = true,
    val backgroundRestricted: Boolean = false,
    val autoRevokeOff: Boolean? = null,
    val notificationsAllowed: Boolean = true,
    val samsung: Boolean = false,
    val lastStop: LastStop? = null,
)

data class LastStop(val reason: Int, val at: Instant)

enum class PhoneFix { Battery, AutoRevoke, Notifications, SleepingApps, ScreenLock }

/** One setting Android can report on. [fix] is the Android page that changes it, offered only when it is not as wanted. */
data class CheckRow(val title: String, val value: String, val ok: Boolean, val fix: PhoneFix?)

data class KeepPage(val noticeTitle: String, val notice: String, val warn: Boolean, val checked: List<CheckRow>, val sleepingApps: Boolean, val history: FactRow)

data class ChannelFacts(val id: String, val name: String, val importance: Int, val sound: Boolean)

/** [showDetails] is the owner's stored choice; [secure] is whether the phone has a screen lock, without which details are never posted; [upgraderNote] shows the one-time card. */
data class NotifFacts(
    val allowed: Boolean = true,
    val channels: List<ChannelFacts> = emptyList(),
    val showDetails: Boolean = false,
    val secure: Boolean = true,
    val upgraderNote: Boolean = false,
    val needsImportance: Int = 4,
    val server: ServerFacts = ServerFacts(),
)

/** The owner's HQ on a server this phone is signed in to for its questions: its address, and whether it signed the phone out. */
data class ServerFacts(val address: String? = null, val signedOut: Boolean = false, val keyLost: Boolean = false, val moved: Boolean = false) {
    val connected: Boolean get() = address != null && !signedOut && !keyLost
}

/** [SettingsUi.busy] while the phone signs in to a server HQ. */
internal const val SERVER_BUSY = "server-hq"

/** What Capture and the widgets are set to, and what is saved on the phone but not yet sent. */
data class CaptureFacts(val shareOpens: String = "note", val showTitles: Boolean = false, val waiting: List<example.hq.spike.capture.Queued> = emptyList(), val smartLinks: Boolean = true)

/** The phone pages under Settings (Keep running, Notifications): only what Android can say, and where it cannot, saying so. */
object PhoneWords {
    private const val HUB_CHANNEL = AlertPoster.HUB
    private val NUMBER_WORDS = listOf(R.string.settings_num_0, R.string.settings_num_1, R.string.settings_num_2, R.string.settings_num_3, R.string.settings_num_4, R.string.settings_num_5)
    private val ORDINALS = listOf(R.string.settings_ord_0, R.string.settings_ord_1, R.string.settings_ord_2, R.string.settings_ord_3, R.string.settings_ord_4, R.string.settings_ord_5, R.string.settings_ord_6)

    fun keepRunning(w: example.hq.spike.ui.shell.Words, f: KeepFacts, now: Instant): KeepPage {
        val battery = w.s(R.string.settings_check_battery)
        val revoke = w.s(R.string.settings_check_autorevoke)
        val rows = buildList {
            add(
                when {
                    f.backgroundRestricted -> CheckRow(battery, w.s(R.string.settings_check_restricted), false, PhoneFix.Battery)
                    f.batteryUnrestricted -> CheckRow(battery, w.s(R.string.settings_check_unrestricted), true, null)
                    else -> CheckRow(battery, w.s(R.string.settings_check_optimised), false, PhoneFix.Battery)
                },
            )
            f.autoRevokeOff?.let { off ->
                add(if (off) CheckRow(revoke, w.s(R.string.settings_check_off_for_app), true, null) else CheckRow(revoke, w.s(R.string.settings_check_on_for_app), false, PhoneFix.AutoRevoke))
            }
            val notifications = w.s(R.string.row_notifications)
            add(if (f.notificationsAllowed) CheckRow(notifications, w.s(R.string.settings_check_allowed), true, null) else CheckRow(notifications, w.s(R.string.settings_check_off_android), false, PhoneFix.Notifications))
        }
        val bad = rows.count { !it.ok }
        val good = rows.size - bad
        val (title, body) = when {
            bad > 0 -> w.n(R.plurals.settings_one_thing_to_fix, bad, bad) to w.n(R.plurals.settings_may_stop_until, bad)
            f.samsung -> w.s(R.string.settings_samsung_title) to w.s(R.string.settings_samsung_body, NUMBER_WORDS.getOrNull(good)?.let(w::s) ?: "$good", ORDINALS.getOrNull(good + 1)?.let(w::s) ?: w.s(R.string.settings_ord_next))
            else -> w.s(R.string.settings_all_fine_title) to w.s(R.string.settings_all_fine_body)
        }
        return KeepPage(title, body, bad > 0 || f.samsung, rows, f.samsung, history(w, f.lastStop, now))
    }

    /** The Home row for Capture: where a share opens, and how many saved things still wait for HQ. */
    fun captureSummary(w: example.hq.spike.ui.shell.Words, f: CaptureFacts): String {
        val opens = w.s(when (f.shareOpens) { "task" -> R.string.settings_share_task; example.hq.spike.capture.CapturePrefs.LAST -> R.string.settings_share_last_used; else -> R.string.settings_share_note })
        return if (f.waiting.isEmpty()) opens else w.s(R.string.settings_share_waiting, f.waiting.size)
    }

    /** What a queued item says about itself: what it is, and why it is still here. */
    fun queuedLine(w: example.hq.spike.ui.shell.Words, q: example.hq.spike.capture.Queued): String = w.s(if (q.isNote) R.string.settings_capture_note else R.string.tasks_task) + " \u00b7 " + (q.failure ?: w.s(R.string.settings_waiting_for_hq))

    /** The Home row: "Good" when every setting Android can read is fine, with the one it cannot read called out on a Samsung. */
    fun keepSummary(w: example.hq.spike.ui.shell.Words, f: KeepFacts, now: Instant): String {
        val page = keepRunning(w, f, now)
        val bad = page.checked.count { !it.ok }
        return when {
            bad > 0 -> w.s(R.string.settings_to_fix, bad)
            f.samsung -> w.s(R.string.settings_good_one_check)
            else -> w.s(R.string.settings_good)
        }
    }

    private fun history(w: example.hq.spike.ui.shell.Words, last: LastStop?, now: Instant): FactRow {
        val title = w.s(R.string.settings_last_stopped)
        if (last == null) return FactRow(title, w.s(R.string.settings_nothing_recorded))
        return FactRow(title, "${SettingsWords.ago(w, last.at, now)} · ${exitWords(w, last.reason)}")
    }

    /** True for an exit Android chose, not one the owner or the app asked for. */
    fun androidStopped(reason: Int): Boolean = reason in ANDROID_REASONS

    private val ANDROID_REASONS = setOf(
        ApplicationExitInfo.REASON_LOW_MEMORY, ApplicationExitInfo.REASON_SIGNALED, ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE,
        ApplicationExitInfo.REASON_DEPENDENCY_DIED, ApplicationExitInfo.REASON_OTHER, REASON_FREEZER,
    )

    private const val REASON_FREEZER = 14

    fun exitWords(w: example.hq.spike.ui.shell.Words, reason: Int): String = w.s(
        when (reason) {
            ApplicationExitInfo.REASON_LOW_MEMORY -> R.string.settings_exit_low_memory
            ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE -> R.string.settings_exit_too_much
            REASON_FREEZER -> R.string.settings_exit_frozen
            else -> R.string.settings_exit_killed
        },
    )

    /** The line under a channel's name, from the catalogue; a channel turned off in Android says so. */
    @androidx.annotation.StringRes
    fun channelLineRes(c: ChannelFacts, appOff: Boolean = false): Int = if (appOff || c.importance <= 0) R.string.channel_off_line else when (c.id) {
        AlertPoster.NEEDS -> R.string.channel_needs_line
        AlertPoster.UPDATES -> R.string.channel_updates_line
        AlertPoster.APP_UPDATES -> R.string.channel_app_updates_line
        AlertPoster.DOWNLOADS -> R.string.channel_downloads_line
        else -> R.string.channel_hub_line
    }

    @androidx.annotation.StringRes
    fun channelNameRes(id: String): Int = when (id) {
        AlertPoster.NEEDS -> R.string.channel_needs_name
        AlertPoster.UPDATES -> R.string.channel_updates_name
        AlertPoster.APP_UPDATES -> R.string.channel_app_updates_name
        AlertPoster.DOWNLOADS -> R.string.channel_downloads_name
        else -> R.string.channel_hub_name
    }
}
