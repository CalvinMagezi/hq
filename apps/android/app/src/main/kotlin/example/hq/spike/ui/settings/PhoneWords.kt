package example.hq.spike.ui.settings

import android.app.ApplicationExitInfo
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

enum class PhoneFix { Battery, AutoRevoke, Notifications, SleepingApps }

/** One setting Android can report on. [fix] is the Android page that changes it, offered only when it is not as wanted. */
data class CheckRow(val title: String, val value: String, val ok: Boolean, val fix: PhoneFix?)

data class KeepPage(val noticeTitle: String, val notice: String, val warn: Boolean, val checked: List<CheckRow>, val sleepingApps: Boolean, val history: FactRow)

data class ChannelFacts(val id: String, val name: String, val importance: Int, val sound: Boolean)

data class NotifFacts(val allowed: Boolean = true, val channels: List<ChannelFacts> = emptyList(), val answers: Boolean = true)

/** The phone pages under Settings (Keep running, Notifications): only what Android can say, and where it cannot, saying so. */
object PhoneWords {
    private const val IMPORTANCE_HIGH = 4
    private const val IMPORTANCE_DEFAULT = 3
    private const val IMPORTANCE_LOW = 2
    private const val IMPORTANCE_MIN = 1
    private const val HUB_CHANNEL = "hub"
    private val NUMBER_WORDS = listOf("Zero", "One", "Two", "Three", "Four", "Five")
    private val ORDINALS = listOf("zeroth", "first", "second", "third", "fourth", "fifth", "sixth")

    fun keepRunning(f: KeepFacts, now: Instant): KeepPage {
        val rows = buildList {
            add(
                when {
                    f.backgroundRestricted -> CheckRow("Battery use", "Restricted", false, PhoneFix.Battery)
                    f.batteryUnrestricted -> CheckRow("Battery use", "Unrestricted", true, null)
                    else -> CheckRow("Battery use", "Optimised, so Android may pause it", false, PhoneFix.Battery)
                },
            )
            f.autoRevokeOff?.let { off ->
                add(if (off) CheckRow("Remove permissions if app is unused", "Off for AgentHQ", true, null) else CheckRow("Remove permissions if app is unused", "On for AgentHQ", false, PhoneFix.AutoRevoke))
            }
            add(if (f.notificationsAllowed) CheckRow("Notifications", "Allowed", true, null) else CheckRow("Notifications", "Off in Android", false, PhoneFix.Notifications))
        }
        val bad = rows.count { !it.ok }
        val good = rows.size - bad
        val (title, body) = when {
            bad > 0 -> (if (bad == 1) "One thing to fix" else "${NUMBER_WORDS.getOrElse(bad) { "$bad" }} things to fix") to "Android may stop AgentHQ until ${if (bad == 1) "it is" else "they are"} set."
            f.samsung -> "One thing Android may still stop" to "${NUMBER_WORDS.getOrElse(good) { "$good" }} settings are fine. A ${ORDINALS.getOrElse(good + 1) { "next" }}, in Samsung's own battery menu, cannot be checked from here."
            else -> "Everything Android can report is fine" to "AgentHQ is set to keep running."
        }
        return KeepPage(title, body, bad > 0 || f.samsung, rows, f.samsung, history(f.lastStop, now))
    }

    /** The Home row: "Good" when every setting Android can read is fine, with the one it cannot read called out on a Samsung. */
    fun keepSummary(f: KeepFacts, now: Instant): String {
        val page = keepRunning(f, now)
        val bad = page.checked.count { !it.ok }
        return when {
            bad > 0 -> "$bad to fix"
            f.samsung -> "Good · 1 thing to check"
            else -> "Good"
        }
    }

    private fun history(last: LastStop?, now: Instant): FactRow {
        val title = "Last stopped by Android"
        if (last == null) return FactRow(title, "Nothing recorded")
        return FactRow(title, "${SettingsWords.ago(last.at, now)} · ${exitWords(last.reason)}")
    }

    /** True for an exit Android chose, not one the owner or the app asked for. */
    fun androidStopped(reason: Int): Boolean = reason in ANDROID_REASONS

    private val ANDROID_REASONS = setOf(
        ApplicationExitInfo.REASON_LOW_MEMORY, ApplicationExitInfo.REASON_SIGNALED, ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE,
        ApplicationExitInfo.REASON_DEPENDENCY_DIED, ApplicationExitInfo.REASON_OTHER, REASON_FREEZER,
    )

    private const val REASON_FREEZER = 14

    fun exitWords(reason: Int): String = when (reason) {
        ApplicationExitInfo.REASON_LOW_MEMORY -> "low memory"
        ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE -> "using too much"
        REASON_FREEZER -> "frozen"
        else -> "killed by the system"
    }

    private fun importanceWords(i: Int) = when {
        i >= IMPORTANCE_HIGH -> "High"
        i == IMPORTANCE_DEFAULT -> "Default"
        i == IMPORTANCE_LOW -> "Low"
        i == IMPORTANCE_MIN -> "Min"
        else -> "Off"
    }

    /** "High · sound on · heads-up": how loud a channel is, from what Android says about it. The service channel is required while HQ runs. */
    fun channelLine(c: ChannelFacts): String {
        if (c.importance <= 0) return "Off in Android"
        if (c.id == HUB_CHANNEL) return "${importanceWords(c.importance)} · required while HQ runs"
        val parts = mutableListOf(importanceWords(c.importance))
        if (c.importance >= IMPORTANCE_DEFAULT) parts += if (c.sound) "sound on" else "silent"
        if (c.importance >= IMPORTANCE_HIGH) parts += "heads-up"
        return parts.joinToString(" · ")
    }
}
