package example.hq.spike.ui.chat

import android.provider.Settings
import android.view.HapticFeedbackConstants
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView

/** Short tick on Send and Stop only, skipped when touch feedback or animations are off. */
object ChatHaptics {
    fun shouldTick(touchFeedbackEnabled: Boolean, animatorDurationScale: Float): Boolean =
        touchFeedbackEnabled && animatorDurationScale > 0f
}

@Composable
fun rememberTick(): () -> Unit {
    val view = LocalView.current
    val context = LocalContext.current
    return remember(view, context) {
        {
            val resolver = context.contentResolver
            val touchOn = Settings.System.getInt(resolver, Settings.System.HAPTIC_FEEDBACK_ENABLED, 1) != 0
            val animScale = Settings.Global.getFloat(resolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f)
            if (ChatHaptics.shouldTick(touchOn, animScale)) {
                view.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
            }
        }
    }
}
