package example.hq.spike.ui.firstrun

import android.content.Context
import android.net.Uri
import android.system.Os
import example.hq.spike.HubService
import example.hq.spike.ui.startup.StartupWords
import java.io.File
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** What unlocks the backup. Held in memory only; the passphrase reaches `hq restore` through a private file, never its command line. */
sealed interface BackupSecret {
    class Passphrase(val text: String) : BackupSecret
    class KeyFile(val uri: Uri) : BackupSecret
}

sealed interface RestoreOutcome {
    data object Done : RestoreOutcome
    class Failed(val reason: String) : RestoreOutcome
}

/** The pure parts of a restore: the command, its environment and the checks before it. */
object RestoreLogic {
    const val MIN_PASSPHRASE = 10
    const val FILE_MODE = 384 // 0600: hq refuses a key file anyone else can read

    fun passphraseProblem(text: String): String? = if (text.trim().length < MIN_PASSPHRASE) FirstRunWords.ERR_PASSPHRASE_SHORT else null

    /**
     * The whole command line. It names files only: the passphrase or key is in [secret], so it can never show in a
     * process list or a crash report.
     */
    fun args(lib: String, archive: String, into: String, secret: String, secretIsKeyFile: Boolean): List<String> =
        listOf(lib, "restore", archive, "--into", into, if (secretIsKeyFile) "--key-file" else "--passphrase-file", secret)

    /** The child sees only what it needs: nothing from the app's environment, and no credential. */
    fun env(home: String, tmp: String, config: String, androidRoot: String?, androidData: String?): Map<String, String> = buildMap {
        put("HOME", home)
        put("TMPDIR", tmp)
        put("HQ_CONFIG_PATH", config)
        androidRoot?.let { put("ANDROID_ROOT", it) }
        androidData?.let { put("ANDROID_DATA", it) }
    }

    /** The first line of a passphrase file is the passphrase. */
    fun passphraseFileText(text: String): String = text.trim() + "\n"

    /** The last line the child printed, shortened and with token-like runs blanked, as a reason a person can read. */
    fun reason(output: List<String>): String = StartupWords.cleanLog(output.filter { it.isNotBlank() }, keep = 1).firstOrNull().orEmpty()
}

/**
 * Restores a `.hqbak` into a fresh folder beside the live vault, then swaps it in. hq is held stopped for the swap,
 * so nothing has the old vault open. A failed restore leaves the live vault as it was.
 */
object Restore {
    private const val WAIT_MINUTES = 30L
    private const val BUFFER_BYTES = 64 * 1024
    private const val OUTPUT_TAIL = 8
    private const val WORK = "restore"
    private const val ARCHIVE = "backup.hqbak"
    private const val SECRET = "secret"
    private const val OUTPUT = "output.txt"

    suspend fun run(context: Context, archive: Uri, secret: BackupSecret): RestoreOutcome = withContext(Dispatchers.IO) {
        val app = context.applicationContext
        val work = File(app.cacheDir, WORK)
        val home = File(app.filesDir, "hqhome")
        val into = File(home, ".vault-restoring")
        val held = HubService.holdHq()
        try {
            if (!held) return@withContext RestoreOutcome.Failed("HQ would not stop. Try again in a minute.")
            wipe(work)
            work.mkdirs()
            copy(app, archive, File(work, ARCHIVE))
            val secretFile = File(work, SECRET)
            secretFile.createNewFile()
            Os.chmod(secretFile.path, RestoreLogic.FILE_MODE)
            when (secret) {
                is BackupSecret.Passphrase -> secretFile.writeText(RestoreLogic.passphraseFileText(secret.text))
                is BackupSecret.KeyFile -> copy(app, secret.uri, secretFile)
            }
            wipe(into)
            val lib = File(app.applicationInfo.nativeLibraryDir, "libhq.so")
            val args = RestoreLogic.args(lib.path, File(work, ARCHIVE).path, into.path, secretFile.path, secret is BackupSecret.KeyFile)
            val output = File(work, OUTPUT)
            val pb = ProcessBuilder(args).redirectErrorStream(true).redirectOutput(output)
            pb.environment().apply {
                clear()
                putAll(RestoreLogic.env(home.path, app.cacheDir.path, File(home, "config.yaml").path, System.getenv("ANDROID_ROOT"), System.getenv("ANDROID_DATA")))
            }
            val child = pb.start()
            child.outputStream.close()
            if (!child.waitFor(WAIT_MINUTES, TimeUnit.MINUTES)) {
                child.destroyForcibly()
                wipe(into)
                return@withContext RestoreOutcome.Failed("It took too long.")
            }
            if (child.exitValue() != 0) {
                val reason = RestoreLogic.reason(output.readLines().takeLast(OUTPUT_TAIL))
                wipe(into)
                return@withContext RestoreOutcome.Failed(reason)
            }
            if (!swap(home, into)) {
                wipe(into)
                return@withContext RestoreOutcome.Failed("The restored files could not be put in place.")
            }
            RestoreOutcome.Done
        } catch (e: IOException) {
            wipe(into)
            RestoreOutcome.Failed("A file could not be read or written.")
        } catch (e: android.system.ErrnoException) {
            wipe(into)
            RestoreOutcome.Failed("A private file could not be made.")
        } finally {
            wipe(work)
            if (held) HubService.releaseHq()
        }
    }

    /** Puts the restored folder where the vault lives, keeping the old one until the new one is in place. */
    private fun swap(home: File, restored: File): Boolean {
        val vault = File(home, ".vault")
        val old = File(home, ".vault-before-restore")
        wipe(old)
        if (vault.exists() && !vault.renameTo(old)) return false
        if (!restored.renameTo(vault)) {
            if (old.exists()) old.renameTo(vault)
            return false
        }
        wipe(old)
        return true
    }

    private fun copy(context: Context, from: Uri, to: File) {
        val input = context.contentResolver.openInputStream(from) ?: throw IOException("cannot open")
        input.use { source ->
            to.outputStream().use { out ->
                val buffer = ByteArray(BUFFER_BYTES)
                while (true) {
                    val n = source.read(buffer)
                    if (n < 0) break
                    out.write(buffer, 0, n)
                }
            }
        }
    }

    /** Deletes a file or folder. A secret file is overwritten with zeros first, so its bytes do not outlive it. */
    private fun wipe(file: File) {
        if (!file.exists()) return
        if (file.isDirectory) {
            file.listFiles()?.forEach(::wipe)
        } else if (file.name == SECRET) {
            runCatching { file.writeBytes(ByteArray(file.length().toInt())) }
        }
        file.delete()
    }
}
