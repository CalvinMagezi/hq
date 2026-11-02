package example.hq.spike.voice

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * One recogniser, one voice and at most one model download per process. Loading both engines takes about ten
 * seconds on a budget phone, so they are kept once loaded; the download outlives the sheet that started it.
 */
// ponytail: engines held until the process ends (about 60 MB); release on onTrimMemory if memory pressure shows up.
object VoiceEngines {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    @Volatile private var ears: Ears? = null
    @Volatile private var mouth: Mouth? = null
    private var download: Deferred<Unit>? = null
    private val _progress = MutableStateFlow<Float?>(null)
    /** 0 to 1 while the models download, null otherwise. */
    val progress: StateFlow<Float?> = _progress

    fun ears(context: Context): Ears = ears ?: synchronized(this) {
        ears ?: Ears(context.applicationContext).also { ears = it }
    }

    fun mouth(context: Context): Mouth = mouth ?: synchronized(this) {
        mouth ?: Mouth(context.applicationContext, scope).also { mouth = it }
    }

    /** The download in progress, if any, so a sheet opened mid-download follows it instead of offering Get again. */
    fun downloading(): Deferred<Unit>? = synchronized(this) { download?.takeIf { it.isActive } }

    /** The running download, or a new one: closing the sheet stops waiting for it, not the download itself. */
    fun download(context: Context): Deferred<Unit> = synchronized(this) {
        download?.takeIf { it.isActive } ?: scope.async {
            try {
                VoiceModels(context.applicationContext).fetch { _progress.value = it }
            } finally {
                _progress.value = null
            }
        }.also { download = it }
    }
}
