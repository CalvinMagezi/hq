package example.hq.spike.ui.firstrun

import android.content.Context
import example.hq.spike.KeyVault
import example.hq.spike.ui.startup.FirstLaunch
import android.database.sqlite.SQLiteDatabase
import java.io.File
import java.io.IOException

/** The screens of the first-run flow. [Restore] hangs off [Welcome] and ends at [Notify], which is skipped when notifications already reach the owner. [Moved] opens the flow on a phone the vault was copied to. */
enum class Step { Welcome, Moved, Model, Chat, Notify, Start, Restore }

/** The pure rules: which screen shows on launch, and where each button leads. */
object FirstRunLogic {
    /**
     * The screen to show now, or null when the flow is over.
     * A finished flow stays finished. A saved step is resumed even after a kill, and wins over a stored key (the key
     * is sealed at step 1, so a later step has one already). With nothing saved, a stored model key or an app that
     * had already heard HQ answer before this flow existed (an upgrade, or a development build set up with adb)
     * means there is nothing to ask. Only a clean install starts at [Step.Welcome].
     */
    fun decide(done: Boolean, saved: Step?, hasModelKey: Boolean, heardHqBefore: Boolean): Step? = when {
        done -> null
        saved != null -> saved
        hasModelKey || heardHqBefore -> null
        else -> Step.Welcome
    }

    fun next(step: Step): Step = when (step) {
        Step.Welcome -> Step.Model
        Step.Moved -> Step.Model
        Step.Model -> Step.Chat
        Step.Chat -> Step.Notify
        Step.Notify -> Step.Start
        Step.Start -> Step.Start
        Step.Restore -> Step.Notify
    }

    fun back(step: Step, moved: Boolean = false): Step = when (step) {
        Step.Welcome -> Step.Welcome
        Step.Moved -> Step.Moved
        Step.Model -> if (moved) Step.Moved else Step.Welcome
        Step.Chat -> Step.Model
        Step.Notify -> Step.Chat
        Step.Start -> Step.Notify
        Step.Restore -> Step.Welcome
    }
}

/** The one thing kept across a kill: which step the user was on, and whether the flow is finished. Never a credential. */
class FirstRunStore(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    fun done(): Boolean = prefs.getBoolean(DONE, false)
    private val appContext = context.applicationContext

    /**
     * The saved step. With none saved, a vault that arrived by phone-to-phone copy opens the [Step.Moved] screen
     * (and is remembered as moved); any other start leaves the install marker behind, so only a copy is ever taken for moved.
     */
    fun step(): Step? {
        prefs.getString(STEP, null)?.let { name -> Step.entries.firstOrNull { it.name == name }?.let { return it } }
        if (Moved.detect(appContext, hasRecord = done() || FirstLaunch.isDone(appContext) || KeyVault.hasModelKey(appContext))) {
            prefs.edit().putBoolean(MOVED, true).apply()
            return Step.Moved.also(::save)
        }
        if (!moved()) InstallMarker.write(appContext)
        return null
    }

    /** True from the moment a copied vault is found until the flow ends or the owner starts fresh. */
    fun moved(): Boolean = prefs.getBoolean(MOVED, false)
    fun save(step: Step) { prefs.edit().putString(STEP, step.name).apply() }
    fun finish() {
        prefs.edit().putBoolean(DONE, true).putBoolean(MOVED, false).remove(STEP).apply()
        InstallMarker.write(appContext)
    }

    /** The owner chose to start fresh: the flow begins again as on a clean install. */
    fun forgetMoved() {
        prefs.edit().putBoolean(MOVED, false).remove(STEP).apply()
        InstallMarker.write(appContext)
    }

    private companion object {
        const val FILE = "firstrun"
        const val STEP = "step"
        const val DONE = "done"
        const val MOVED = "moved"
    }
}

/** The file in `noBackupFilesDir` that Android never copies: its absence next to a vault means the vault came from another phone. */
object InstallMarker {
    const val NAME = "install.marker"

    fun file(context: Context): File = File(context.noBackupFilesDir, NAME)
    fun exists(context: Context): Boolean = file(context).exists()

    fun write(context: Context) {
        val f = file(context)
        if (f.exists()) return
        try {
            f.parentFile?.mkdirs()
            f.writeText("1")
        } catch (_: IOException) {
            // Without the marker the next start only asks the question again.
        }
    }
}

/** Rule 2 of the moving-phones spec, as plain functions. */
object Moved {
    const val VAULT = "hqhome/.vault"

    /** Moved: a vault, no first-run record and no marker. An upgrader has the record, so is never taken for moved. */
    fun isMoved(vaultExists: Boolean, hasRecord: Boolean, markerExists: Boolean): Boolean = vaultExists && !hasRecord && !markerExists

    fun vault(context: Context): File = File(context.filesDir, VAULT)

    fun detect(context: Context, hasRecord: Boolean): Boolean = isMoved(vault(context).isDirectory, hasRecord, InstallMarker.exists(context))
}

/** What came with a copied vault, read from files and the database only. Never a key or an address. */
class MovedFacts(val notes: Int, val tasks: Int, val service: String?, val databaseOk: Boolean)

/** Reading, checking and clearing a copied vault. The pure parts are plain functions so they can be tested without a phone. */
object MovedPhone {
    const val DB = "_data/vault.db"
    const val WAIT_SECONDS = 120L

    fun args(lib: String): List<String> = listOf(lib, "vault", "moved")

    fun env(home: String, tmp: String, config: String, vault: String, androidRoot: String?, androidData: String?): Map<String, String> =
        RestoreLogic.env(home, tmp, config, androidRoot, androidData) + ("HQ_VAULT_PATH" to vault)

    /** The AI service named by the moved database's backend, or null when it names none this app offers. */
    fun serviceFor(backend: String?): String? {
        val b = backend?.lowercase() ?: return null
        return Provider.entries.firstOrNull { it != Provider.Other && b.contains(it.name.lowercase()) }?.label
    }

    /** Counts note files, skipping hidden and underscore folders (the database and caches live there). */
    fun countNotes(vault: File): Int = vault.walkTopDown()
        .onEnter { !it.name.startsWith(".") && !it.name.startsWith("_") || it == vault }
        .count { it.isFile && it.extension.equals("md", ignoreCase = true) }

    /** SQLite's quick check. A missing database is fine: hq makes a new one. */
    fun databaseOk(db: File): Boolean {
        if (!db.isFile) return true
        return try {
            SQLiteDatabase.openDatabase(db.path, null, SQLiteDatabase.OPEN_READONLY).use { d ->
                d.rawQuery("PRAGMA quick_check", null).use { c -> c.moveToFirst() && c.getString(0) == "ok" }
            }
        } catch (_: Exception) {
            false
        }
    }

    fun facts(vault: File): MovedFacts {
        val db = File(vault, DB)
        val ok = databaseOk(db)
        var tasks = 0
        var backend: String? = null
        if (ok && db.isFile) runCatching {
            SQLiteDatabase.openDatabase(db.path, null, SQLiteDatabase.OPEN_READONLY).use { d ->
                d.rawQuery("SELECT count(*) FROM tasks", null).use { if (it.moveToFirst()) tasks = it.getInt(0) }
                runCatching {
                    d.rawQuery("SELECT resolved_backend FROM subagent_runs WHERE resolved_backend IS NOT NULL ORDER BY started_at DESC LIMIT 1", null).use { if (it.moveToFirst()) backend = it.getString(0) }
                }
            }
        }
        return MovedFacts(countNotes(vault), tasks, serviceFor(backend), ok)
    }

    /** Notes only: the database files go, the note files stay, and hq makes a new database. */
    fun dropDatabase(vault: File) {
        listOf(DB, "$DB-wal", "$DB-shm").forEach { File(vault, it).delete() }
    }

    /** Runs `hq vault moved` against [vault] (the live one by default). True when it ended cleanly. */
    fun forget(context: Context, vault: File? = null): Boolean {
        val app = context.applicationContext
        val home = File(app.filesDir, "hqhome")
        val target = vault ?: File(home, ".vault")
        val lib = File(app.applicationInfo.nativeLibraryDir, "libhq.so")
        return try {
            val pb = ProcessBuilder(args(lib.path)).redirectErrorStream(true).redirectOutput(java.io.File(app.cacheDir, "moved.out"))
            pb.environment().apply {
                clear()
                putAll(env(home.path, app.cacheDir.path, File(home, "config.yaml").path, target.path, System.getenv("ANDROID_ROOT"), System.getenv("ANDROID_DATA")))
            }
            val child = pb.start()
            child.outputStream.close()
            if (!child.waitFor(WAIT_SECONDS, java.util.concurrent.TimeUnit.SECONDS)) {
                child.destroyForcibly()
                false
            } else child.exitValue() == 0
        } catch (_: IOException) {
            false
        }
    }
}
