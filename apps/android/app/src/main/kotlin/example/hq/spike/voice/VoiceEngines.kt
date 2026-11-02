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
 * One recogniser and at most one model download per process. Loading the recogniser takes seconds on a budget phone,
 * so it is kept once loaded; the download outlives the sheet that started it.
 */
// ponytail: the recogniser is held until the process ends (about 70 MB); release on onTrimMemory if memory pressure shows up.
object VoiceEngines {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    @Volatile private var ears: Ears? = null
    private var download: Deferred<Unit>? = null
    private val _progress = MutableStateFlow<Float?>(null)
    /** 0 to 1 while the models download, null otherwise. */
    val progress: StateFlow<Float?> = _progress

    fun ears(context: Context): Ears = ears ?: synchronized(this) {
        ears ?: Ears(context.applicationContext).also { ears = it }
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
