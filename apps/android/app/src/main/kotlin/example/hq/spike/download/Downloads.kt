package example.hq.spike.download

import android.content.Context
import android.net.ConnectivityManager
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * Every large download of this app, by id, over the one engine. The hub starts it (the network callback and the saved
 * requests), the screens and the notices' buttons act through it, and it holds the owner's "ask on any network" choice.
 */
object Downloads {
    private const val PREFS = "downloads"
    private const val ASK_ANY = "ask_any_network"
    private const val REMOVED = "removed"

    private val engines = ConcurrentHashMap<String, LargeDownload>()
    private val flow = MutableStateFlow<Map<String, DlState>>(emptyMap())
    val states: StateFlow<Map<String, DlState>> = flow
    private val listeners = CopyOnWriteArrayList<(Item, DlState) -> Unit>()
    @Volatile private var started = false
    private var callback: ConnectivityManager.NetworkCallback? = null

    /** Something to do on every change of any download, for example the update's install after its file is checked. */
    fun listen(l: (Item, DlState) -> Unit) { listeners.add(l) }

    fun askAnyNetwork(c: Context): Boolean = prefs(c).getBoolean(ASK_ANY, false)

    fun setAskAnyNetwork(c: Context, on: Boolean) = prefs(c).edit().putBoolean(ASK_ANY, on).apply()

    /** At the hub's start: the network callback, and each saved request back as it was, with old partial files removed. */
    @Synchronized fun start(c: Context) {
        if (started) return
        started = true
        val app = c.applicationContext
        val dir = AndroidDownloadEnv.dir(app)
        dir.listFiles { f -> f.name.endsWith(".json") }?.forEach { meta -> LargeDownload.readItem(meta)?.let { engine(app, it).restore() } }
        val now = System.currentTimeMillis()
        dir.listFiles { f -> f.name.endsWith(".part") }?.filter { DownloadRules.expired(it.lastModified(), now) }?.forEach { it.delete() }
        purgeRemoved(app)
        callback = AndroidDownloadEnv(app).watch { Thread({ engines.values.forEach { it.networkChanged() } }, "download-net").start() }
    }

    /** The engine for [item], made on first use; a different file under the same id replaces the old one. */
    @Synchronized fun engine(c: Context, item: Item): LargeDownload {
        val app = c.applicationContext
        engines[item.id]?.let { if (it.item == item) return it else it.discard() }
        val made = LargeDownload(item, AndroidDownloadEnv.dir(app), AndroidDownloadEnv(app)) { i, s -> changed(app, i, s) }
        engines[item.id] = made
        flow.value = flow.value + (item.id to made.state.value)
        return made
    }

    /** Where the downloads and their files are kept: not backed up, and not read by other apps. */
    fun dirOf(c: Context): File = AndroidDownloadEnv.dir(c)

    fun get(id: String): LargeDownload? = engines[id]

    private fun changed(c: Context, item: Item, state: DlState) {
        flow.value = flow.value + (item.id to state)
        DownloadNotices.onChange(c, item, state)
        listeners.forEach { it(item, state) }
    }

    /** A button on a notice. The service that carries it is already running, so this only acts. */
    fun act(id: String, op: String) {
        val d = engines[id] ?: return
        Thread({
            when (op) {
                DownloadNotices.OP_PAUSE -> d.pause()
                DownloadNotices.OP_RESUME -> d.resume()
                DownloadNotices.OP_CANCEL -> d.cancel()
                DownloadNotices.OP_UNDO -> d.undo()
            }
        }, "download-act").start()
    }

    /** Bytes of unfinished downloads kept on this phone. */
    fun unfinishedBytes(c: Context): Long = AndroidDownloadEnv.dir(c).listFiles { f -> f.name.endsWith(".part") }?.sumOf { it.length() } ?: 0L

    /** Takes the unfinished downloads aside (Undo puts them back) and resets their rows. */
    @Synchronized fun removeUnfinished(c: Context): Boolean {
        val dir = AndroidDownloadEnv.dir(c)
        val aside = File(dir, REMOVED).apply { mkdirs() }
        val moved = dir.listFiles { f -> f.name.endsWith(".part") || f.name.endsWith(".part.json") || f.name.endsWith(".json") }.orEmpty()
        val parts = moved.filter { it.name.endsWith(".part") }.map { it.name.removeSuffix(".part") }
        if (parts.isEmpty()) return false
        moved.filter { f -> parts.any { f.name == "$it.part" || f.name == "$it.json" } }.forEach { it.renameTo(File(aside, it.name)) }
        engines.values.filter { it.item.dest in parts }.forEach { it.discard() }
        return true
    }

    @Synchronized fun undoRemove(c: Context) {
        val dir = AndroidDownloadEnv.dir(c)
        File(dir, REMOVED).listFiles()?.forEach { it.renameTo(File(dir, it.name)) }
        engines.values.filter { File(dir, it.item.dest + ".part").exists() }.forEach { it.undoKeep() }
    }

    /** Past the Undo window, what was taken aside goes. */
    @Synchronized fun purgeRemoved(c: Context) {
        File(AndroidDownloadEnv.dir(c), REMOVED).deleteRecursively()
    }

    private fun prefs(c: Context) = c.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}
