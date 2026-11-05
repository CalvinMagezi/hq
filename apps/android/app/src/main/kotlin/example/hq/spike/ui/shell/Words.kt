package example.hq.spike.ui.shell

import android.content.res.Resources
import androidx.annotation.PluralsRes
import androidx.annotation.StringRes
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext

/**
 * Reads sentences from the one catalogue for code that builds a line out of facts. Pure functions take a [Words] instead of
 * holding English, so a test reads the same generated resources the screen does.
 */
class Words(private val resources: Resources) {
    fun s(@StringRes id: Int, vararg args: Any): String = resources.getString(id, *args)

    /** A plural: [n] picks the form, [args] fill the placeholders in the order the sentence first names them. */
    fun n(@PluralsRes id: Int, n: Int, vararg args: Any): String = resources.getQuantityString(id, n, *args)
}

@Composable
fun rememberWords(): Words {
    val context = LocalContext.current
    val configuration = LocalConfiguration.current
    return remember(context, configuration) { Words(context.resources) }
}
