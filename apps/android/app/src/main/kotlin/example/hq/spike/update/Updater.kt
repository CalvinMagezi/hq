package example.hq.spike.update

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.util.Log
import java.io.File
import java.io.IOException
import java.security.MessageDigest
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response

/**
 * Updates without the Play Store. The channel manifest is trusted only after `libhq.so update verify-file` accepts its minisign signature
 * (the release key is compiled into that binary); the APK is trusted only because its SHA-256 is in that signed manifest. Android then checks
 * the APK's own signature against the installed one, so a build signed with another key can never replace this app.
 * The owner starts every download and install; a check only sets [state].
 */
object Updater {
    private const val TAG = "hq-update"
    private const val PREFS = "update"
    private const val KEY_CHANNEL = "channel"
    private const val KEY_LAST_CHECK = "last_check"
    private const val DIR = "update"
    private const val APK_NAME = "agent-hq-android.apk"
    private const val VERIFY_WAIT_SECONDS = 20L
    private const val WATCH_POLL_MS = 60L * 60 * 1000
    private const val BUFFER_BYTES = 64 * 1024
    private const val REQUEST_CODE = 7001
    const val ACTION_RESULT = "example.hq.spike.update.RESULT"

    private val flow = MutableStateFlow<UpdateState>(UpdateState.Idle)
    val state: StateFlow<UpdateState> = flow

    private val http = OkHttpClient.Builder()
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(60, TimeUnit.SECONDS)
        .build()

    fun channel(context: Context): UpdateChannel = UpdateChannel.of(prefs(context).getString(KEY_CHANNEL, null))

    fun setChannel(context: Context, channel: UpdateChannel) {
        prefs(context).edit().putString(KEY_CHANNEL, channel.id).putLong(KEY_LAST_CHECK, 0).apply()
        flow.value = UpdateState.Idle
    }

    /** Runs in the service's own thread: asks once a day while it is running, and only records what it found. */
    fun watch(context: Context, alive: () -> Boolean) {
        while (alive()) {
            if (UpdateLogic.due(prefs(context).getLong(KEY_LAST_CHECK, 0), System.currentTimeMillis())) check(context)
            try {
                Thread.sleep(WATCH_POLL_MS)
            } catch (_: InterruptedException) {
                return
            }
        }
    }

    /** Fetches and verifies the channel manifest and settles [state]. Blocking; call it off the main thread. */
    fun check(context: Context) {
        if (flow.value == UpdateState.Checking || flow.value == UpdateState.Downloading) return
        flow.value = UpdateState.Checking
        flow.value = try {
            val verdict = fetchVerdict(context)
            prefs(context).edit().putLong(KEY_LAST_CHECK, System.currentTimeMillis()).apply()
            when (verdict) {
                UpdateVerdict.UpToDate -> UpdateState.UpToDate
                is UpdateVerdict.Available -> UpdateState.Available(verdict.manifest)
                is UpdateVerdict.NeedsNewerAndroid -> UpdateState.NeedsNewerAndroid(verdict.minSdk)
            }
        } catch (e: UpdateException) {
            UpdateState.Failed(e.message ?: "The update check failed.")
        } catch (e: IOException) {
            Log.w(TAG, "check failed: ${e.javaClass.simpleName}")
            UpdateState.Failed("Could not reach the update server.")
        }
    }

    /** Downloads the APK named by the signed manifest, checks its hash, and hands it to Android's installer. Blocking. */
    fun install(context: Context, manifest: ChannelManifest) {
        if (flow.value == UpdateState.Downloading) return
        if (!canInstall(context)) {
            flow.value = UpdateState.Failed("Allow AgentHQ to install apps, then try again.")
            context.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            return
        }
        flow.value = UpdateState.Downloading
        try {
            val apk = download(context, manifest)
            commit(context, apk)
            flow.value = UpdateState.Installing
        } catch (e: UpdateException) {
            flow.value = UpdateState.Failed(e.message ?: "The update failed.")
        } catch (e: IOException) {
            Log.w(TAG, "install failed: ${e.javaClass.simpleName}")
            flow.value = UpdateState.Failed("The download did not finish.")
        }
    }

    /** What Android's installer answered, from [InstallResultReceiver]. Success needs no state: the app is replaced. */
    internal fun finished(status: Int, message: String?) {
        flow.value = UpdateLogic.installOutcome(status, message)
    }

    private fun canInstall(context: Context): Boolean = context.packageManager.canRequestPackageInstalls()

    private fun fetchVerdict(context: Context): UpdateVerdict {
        val url = UpdateLogic.manifestUrl(channel(context))
        val dir = File(context.cacheDir, DIR).apply { mkdirs() }
        // The signature names the file it signed, so the manifest is saved under its release name.
        val manifestFile = File(dir, UpdateLogic.CHANNEL_FILE)
        val sigFile = File(dir, UpdateLogic.CHANNEL_FILE + ".minisig")
        manifestFile.writeBytes(getCapped(url))
        sigFile.writeBytes(getCapped("$url.minisig"))
        if (!signatureHolds(context, manifestFile, sigFile)) throw UpdateException("The update information is not signed by the AgentHQ release key. Nothing was installed.")
        val manifest = UpdateLogic.parse(manifestFile.readText()) ?: throw UpdateException("The update information is not in a form this version understands.")
        return UpdateLogic.verdict(manifest, installedCode(context), Build.VERSION.SDK_INT)
    }

    private fun installedCode(context: Context): Long {
        val info = context.packageManager.getPackageInfo(context.packageName, 0)
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) info.longVersionCode else @Suppress("DEPRECATION") info.versionCode.toLong()
    }

    private fun getCapped(url: String): ByteArray {
        http.newCall(Request.Builder().url(url).build()).execute().use { response ->
            if (!response.isSuccessful) throw UpdateException("The update server answered ${response.code}.")
            val source = response.body.source()
            if (source.request(UpdateLogic.MANIFEST_MAX_BYTES + 1)) throw UpdateException("The update information is too large.")
            return source.readByteArray()
        }
    }

    /** True only when `hq update verify-file` exits 0; any other result, including a missing binary, is a refusal. */
    private fun signatureHolds(context: Context, manifest: File, sig: File): Boolean = try {
        val lib = File(context.applicationInfo.nativeLibraryDir, "libhq.so")
        val pb = ProcessBuilder(lib.path, "update", "verify-file", manifest.path, sig.path).redirectErrorStream(true)
        // Nothing from the app's environment: the check reads two files and the key compiled into the binary.
        pb.environment().apply {
            clear()
            put("HOME", File(context.filesDir, "hqhome").apply { mkdirs() }.path)
            put("TMPDIR", context.cacheDir.path)
            System.getenv("ANDROID_ROOT")?.let { put("ANDROID_ROOT", it) }
            System.getenv("ANDROID_DATA")?.let { put("ANDROID_DATA", it) }
        }
        val child = pb.start()
        child.outputStream.close()
        if (!child.waitFor(VERIFY_WAIT_SECONDS, TimeUnit.SECONDS)) {
            child.destroyForcibly()
            false
        } else {
            child.exitValue() == 0
        }
    } catch (e: IOException) {
        Log.w(TAG, "verify-file could not run: ${e.javaClass.simpleName}")
        false
    }

    private fun download(context: Context, manifest: ChannelManifest): File {
        val dir = File(context.cacheDir, DIR).apply { mkdirs() }
        val apk = File(dir, APK_NAME)
        apk.delete()
        val digest = MessageDigest.getInstance("SHA-256")
        http.newCall(Request.Builder().url(manifest.url).build()).execute().use { response: Response ->
            if (!response.isSuccessful) throw UpdateException("The update server answered ${response.code}.")
            val length = response.body.contentLength()
            if (length > UpdateLogic.APK_MAX_BYTES) throw UpdateException("The update is larger than expected. Nothing was installed.")
            var total = 0L
            response.body.byteStream().use { input ->
                apk.outputStream().use { out ->
                    val buffer = ByteArray(BUFFER_BYTES)
                    while (true) {
                        val n = input.read(buffer)
                        if (n < 0) break
                        total += n
                        if (total > UpdateLogic.APK_MAX_BYTES) throw UpdateException("The update is larger than expected. Nothing was installed.")
                        digest.update(buffer, 0, n)
                        out.write(buffer, 0, n)
                    }
                }
            }
        }
        val hex = digest.digest().joinToString("") { "%02x".format(it) }
        if (hex != manifest.sha256) {
            apk.delete()
            throw UpdateException("The download does not match the signed update information. Nothing was installed.")
        }
        return apk
    }

    private fun commit(context: Context, apk: File) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
        params.setSize(apk.length())
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            apk.inputStream().use { input -> session.openWrite(APK_NAME, 0, apk.length()).use { out -> input.copyTo(out, BUFFER_BYTES); session.fsync(out) } }
            val result = Intent(ACTION_RESULT).setClass(context, InstallResultReceiver::class.java).setPackage(context.packageName)
            // Mutable because the installer adds its status to the intent.
            val pending = PendingIntent.getBroadcast(context, REQUEST_CODE, result, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE)
            session.commit(pending.intentSender)
        }
    }

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private class UpdateException(message: String) : Exception(message)
}
