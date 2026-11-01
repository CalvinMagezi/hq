package example.hq.spike.voice

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

/**
 * One recogniser and one voice per process. Loading both takes about ten seconds on a budget phone, so they are
 * kept once loaded and every request after the first starts listening at once.
 */
// ponytail: held until the process ends (about 60 MB); release on onTrimMemory if memory pressure shows up.
object VoiceEngines {
    @Volatile private var ears: Ears? = null
    @Volatile private var mouth: Mouth? = null

    fun ears(context: Context): Ears = ears ?: synchronized(this) {
        ears ?: Ears(context.applicationContext).also { ears = it }
    }

    fun mouth(context: Context): Mouth = mouth ?: synchronized(this) {
        mouth ?: Mouth(context.applicationContext, CoroutineScope(SupervisorJob() + Dispatchers.Default)).also { mouth = it }
    }
}
