package example.hq.spike.ui.shell

import android.content.res.Resources
import androidx.annotation.PluralsRes
import androidx.annotation.StringRes
import android.text.format.DateFormat
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

/**
 * Reads sentences from the one catalogue for code that builds a line out of facts. Pure functions take a [Words] instead of
 * holding English, so a test reads the same generated resources the screen does.
 */
class Words(private val resources: Resources, private val is24Hour: Boolean? = null) {
    companion object {
        fun of(context: android.content.Context) = Words(context.resources, DateFormat.is24HourFormat(context))
    }

    fun s(@StringRes id: Int, vararg args: Any): String = resources.getString(id, *args)

    /** A plural: [n] picks the form, [args] fill the placeholders in the order the sentence first names them. */
    fun n(@PluralsRes id: Int, n: Int, vararg args: Any): String = resources.getQuantityString(id, n, *args)

    /** The time of day for something from today, and "Tue 3 Nov" for anything older. */
    fun clockOrDay(at: Instant, now: Instant = Instant.now(), zone: ZoneId = ZoneId.systemDefault()): String {
        if (at.atZone(zone).toLocalDate() == now.atZone(zone).toLocalDate()) return clock(at, zone)
        val locale = Locale.getDefault()
        return at.atZone(zone).format(DateTimeFormatter.ofPattern(DateFormat.getBestDateTimePattern(locale, "EEEdMMM"), locale))
    }

    /** A time of day in the phone's own 12 or 24-hour setting: 2:32 PM or 14:32. */
    fun clock(at: Instant, zone: ZoneId = ZoneId.systemDefault()): String {
        val locale = Locale.getDefault()
        val skeleton = when (is24Hour) { true -> "Hm"; false -> "hm"; null -> null }
        val formatter = if (skeleton == null) DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT) else DateTimeFormatter.ofPattern(DateFormat.getBestDateTimePattern(locale, skeleton), locale)
        return at.atZone(zone).format(formatter.withLocale(locale))
    }
}

@Composable
fun rememberWords(): Words {
    val context = LocalContext.current
    val configuration = LocalConfiguration.current
    return remember(context, configuration) { Words.of(context) }
}
