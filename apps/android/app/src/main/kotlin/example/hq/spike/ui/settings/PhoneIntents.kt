package example.hq.spike.ui.settings

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings

/** The Android pages that change how the system treats this app, tried in order until one opens. */
object PhoneIntents {
    private const val SAMSUNG_BATTERY_PACKAGE = "com.samsung.android.lool"
    private const val SAMSUNG_BATTERY_PAGE = "com.samsung.android.sm.battery.ui.BatteryActivity"

    fun tries(c: Context, fix: PhoneFix, backgroundRestricted: Boolean = false): List<Intent> {
        val details = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${c.packageName}"))
        return when (fix) {
            PhoneFix.Battery -> if (backgroundRestricted) listOf(details) else listOf(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS), details)
            PhoneFix.AutoRevoke -> listOf(Intent(Intent.ACTION_AUTO_REVOKE_PERMISSIONS, Uri.parse("package:${c.packageName}")), details)
            PhoneFix.Notifications -> listOf(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, c.packageName), details)
            PhoneFix.SleepingApps -> listOf(Intent().setClassName(SAMSUNG_BATTERY_PACKAGE, SAMSUNG_BATTERY_PAGE), Intent(Settings.ACTION_BATTERY_SAVER_SETTINGS), details)
        }
    }

    /** True once one of [tries] opened; a page this phone does not have, or one that is not allowed, is passed over. */
    fun startFirst(c: Context, tries: List<Intent>): Boolean {
        for (i in tries) {
            try { c.startActivity(i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); return true } catch (e: ActivityNotFoundException) { continue } catch (e: SecurityException) { continue }
        }
        return false
    }
}
