package example.hq.spike.update

import example.hq.spike.download.DlState
import example.hq.spike.download.Downloads
import example.hq.spike.download.Item
import example.hq.spike.measure.DataPurpose
import example.hq.spike.R
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
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
import example.hq.spike.HubService
import example.hq.spike.voice.VoiceEngines

/**
 * Updates without the Play Store. The channel manifest is trusted only after `libhq.so update verify-file` accepts its minisign signature
 * (the release key is compiled into that binary); the APK is trusted only because its SHA-256 is in that signed manifest. Android then checks
 * the APK's own signature against the installed one, so a build signed with another key can never replace this app.
 * The owner starts every download and install; a check only sets [state]. The APK comes over the shared download engine, so it asks
 * before spending mobile data, resumes after a cut, and is checked against the signed size and hash before anything uses it.
 */
object Updater {
    private const val TAG = "hq-update"
    private const val PREFS = "update"
    private const val KEY_CHANNEL = "channel"
    private const val KEY_LAST_CHECK = "last_check"
    private const val DIR = "update"
    private const val APK_NAME = "agent-hq-android.apk"
    private const val ITEM_ID = "update"
    private const val VERIFY_WAIT_SECONDS = 20L
    private const val FIRST_CHECK_MS = 10L * 60 * 1000
    private const val WATCH_POLL_MS = 60L * 60 * 1000
    private const val PAGE_CHECK_MS = 60L * 60 * 1000
    private const val SNOOZE_MS = 7L * 24 * 60 * 60 * 1000
    private const val DAY_MS = 24L * 60 * 60 * 1000
    private const val BUFFER_BYTES = 64 * 1024
    private const val REQUEST_CODE = 7001
    private const val KEY_SEQ = "highest_seq"
    private const val KEY_BUCKET = "bucket"
    private const val KEY_OFFERED = "offered_code"
    private const val KEY_SAVE_COPY = "switch_save_copy"
    private const val KEY_PENDING = "pending_code"
    private const val KEY_INSTALLING = "installing_code"
    private const val KEY_INSTALLING_NAME = "installing_name"
    private const val KEY_INSTALLED_AT = "installed_at"
    private const val KEY_INSTALLED_NAME = "installed_name"
    private const val KEY_OPENED = "opened_code"
    private const val KEY_SNOOZE_CODE = "snooze_code"
    private const val KEY_SNOOZE_UNTIL = "snooze_until"
    private const val KEY_N_CODE = "notice_code"
    private const val KEY_N_FIRST = "notice_first"
    private const val KEY_N_LAST = "notice_last"
    private const val KEY_W_NOTICED = "withdrawn_noticed"
    private const val BUCKETS = 100
    private val BUSY_PORT = example.hq.spike.BuildConfig.HQ_PORT
    const val ACTION_RESULT = "example.hq.spike.update.RESULT"

    /** A line the owner sees once: [id] is a catalogue sentence and [arg] fills its one placeholder. */
    data class Line(val id: Int, val arg: String? = null)

    /** What the Update page and the About rows show beyond [state]. */
    data class Facts(
        val installedVersion: String = "",
        val installedBuild: Long = 0,
        val release: String = "",
        val checkedAt: Long = 0,
        val installedToday: String? = null,
        val canInstall: Boolean = true,
    )

    private val flow = MutableStateFlow<UpdateState>(UpdateState.Idle)
    val state: StateFlow<UpdateState> = flow

    /** The blue dot on the gear: an update is ready and its page has not been opened. */
    private val dotFlow = MutableStateFlow(false)
    val dot: StateFlow<Boolean> = dotFlow

    /** True while "Not now" hides the dot and the card for this version. */
    private val hiddenFlow = MutableStateFlow(false)
    val hidden: StateFlow<Boolean> = hiddenFlow

    private val lineFlow = MutableStateFlow<Line?>(null)
    val line: StateFlow<Line?> = lineFlow

    fun clearLine() { lineFlow.value = null }

    @Volatile private var app: Context? = null
    @Volatile private var current: ChannelManifest? = null
    @Volatile private var forceStop = false
    @Volatile private var listening = false

    private val http = OkHttpClient.Builder()
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(60, TimeUnit.SECONDS)
        .build()

    fun channel(context: Context): UpdateChannel = UpdateChannel.of(prefs(context).getString(KEY_CHANNEL, null))

    fun setChannel(context: Context, channel: UpdateChannel) {
        prefs(context).edit().putString(KEY_CHANNEL, channel.id).putLong(KEY_LAST_CHECK, 0).remove(KEY_PENDING).apply()
        dropSaved(context)
        set(context, if (UpdateLogic.enabled()) UpdateState.Idle else UpdateState.Off)
    }

    /** Runs in the service's own thread: loads what was saved, first asks ten minutes after the start, then once a day. */
    fun watch(context: Context, alive: () -> Boolean) {
        begin(context)
        if (!sleep(FIRST_CHECK_MS)) return
        while (alive()) {
            if (UpdateLogic.due(prefs(context).getLong(KEY_LAST_CHECK, 0), System.currentTimeMillis())) check(context)
            if (!sleep(WATCH_POLL_MS)) return
        }
    }

    private fun sleep(ms: Long): Boolean = try { Thread.sleep(ms); true } catch (_: InterruptedException) { false }

    /** At the hub's start: an install that went through is told, and a checked update saved on disk comes back without the network. */
    fun begin(context: Context) {
        app = context.applicationContext
        listenToDownloads()
        if (!UpdateLogic.enabled()) { set(context, UpdateState.Off); return }
        settleInstalled(context)
        reloadSaved(context)
    }

    private fun listenToDownloads() {
        if (listening) return
        listening = true
        Downloads.listen { item, s -> if (item.id == ITEM_ID) onDownload(s) }
    }

    private fun onDownload(s: DlState) {
        val context = app ?: return
        val m = current ?: return
        when (s) {
            is DlState.Done -> afterDownload(context, m)
            DlState.Ready, is DlState.Cancelled, is DlState.NoSpace -> {
                if (flow.value is UpdateState.Downloading) { prefs(context).edit().remove(KEY_PENDING).apply(); set(context, UpdateState.Available(m)) }
            }
            else -> if (flow.value is UpdateState.Available) set(context, UpdateState.Downloading)
        }
    }

    /** Fetches and verifies the channel manifest and settles [state]. Blocking; call it off the main thread. */
    fun check(context: Context) {
        app = context.applicationContext
        if (!UpdateLogic.enabled()) { set(context, UpdateState.Off); return }
        val before = flow.value
        if (before == UpdateState.Checking || before == UpdateState.Downloading || before is UpdateState.Installing) return
        set(context, UpdateState.Checking)
        try {
            val verdict = fetchVerdict(context)
            prefs(context).edit().putLong(KEY_LAST_CHECK, System.currentTimeMillis()).apply()
            settle(context, verdict, before)
        } catch (e: UpdateException) {
            Log.w(TAG, "check refused: ${e.message}")
            set(context, failedCheck(before))
        } catch (e: IOException) {
            Log.w(TAG, "check failed: ${e.javaClass.simpleName}")
            set(context, failedCheck(before))
        }
    }

    private fun failedCheck(before: UpdateState): UpdateState = if (manifestOf(before) != null) before else UpdateState.Problem(ProblemKind.CheckFailed)

    private fun settle(context: Context, verdict: UpdateVerdict?, before: UpdateState) {
        when (verdict) {
            null -> set(context, if (before is UpdateState.Checking) UpdateState.Idle else before)
            UpdateVerdict.UpToDate -> { dropSaved(context); current = null; set(context, UpdateState.UpToDate) }
            is UpdateVerdict.Available -> offer(context, verdict.manifest)
            is UpdateVerdict.NeedsNewerAndroid -> set(context, UpdateState.NeedsNewerAndroid(verdict.minSdk, verdict.version))
            is UpdateVerdict.InstalledWithdrawn -> {
                val p = prefs(context)
                val code = installedCode(context)
                if (p.getLong(KEY_W_NOTICED, 0) != code) { p.edit().putLong(KEY_W_NOTICED, code).apply(); UpdateNotices.withdrawn(context, installedName(context)) }
                set(context, UpdateState.InstalledWithdrawn(installedName(context), verdict.fix))
            }
            is UpdateVerdict.Pulled -> {
                dropSaved(context)
                Downloads.get(ITEM_ID)?.discard()
                File(Downloads.dirOf(context), APK_NAME).delete()
                prefs(context).edit().remove(KEY_OFFERED).remove(KEY_PENDING).apply()
                UpdateNotices.clear(context)
                current = null
                set(context, UpdateState.Pulled(verdict.version))
            }
        }
    }

    private fun offer(context: Context, m: ChannelManifest) {
        current = m
        val p = prefs(context)
        p.edit().putLong(KEY_OFFERED, m.versionCode).apply()
        val state = if (readyApk(context, m)) UpdateState.Downloaded(m) else UpdateState.Available(m)
        set(context, state)
        notifyOffer(context, m)
    }

    /** One notice when a version is found, then a reminder after a week (an urgent version, each day). */
    private fun notifyOffer(context: Context, m: ChannelManifest) {
        val p = prefs(context)
        val same = p.getLong(KEY_N_CODE, 0) == m.versionCode
        val first = if (same) p.getLong(KEY_N_FIRST, 0) else 0
        val last = if (same) p.getLong(KEY_N_LAST, 0) else 0
        val now = System.currentTimeMillis()
        val kind = UpdateNotices.due(first, last, now, m.urgent) ?: return
        UpdateNotices.found(context, m, kind)
        p.edit().putLong(KEY_N_CODE, m.versionCode).putLong(KEY_N_FIRST, if (first > 0) first else now).putLong(KEY_N_LAST, now).apply()
    }

    internal fun itemOf(m: ChannelManifest) = Item(ITEM_ID, "AgentHQ ${m.version}", m.url, m.size, m.sha256, DataPurpose.AppUpdates, m.size, APK_NAME)

    /**
     * The owner chose Download and install. The file comes over the download engine (which may ask about mobile data); when it is
     * checked, HQ must be free (the idle gate) before Android's installer gets it. Blocking only for the gate; call it off the main thread.
     */
    fun install(context: Context, manifest: ChannelManifest, stopAndInstall: Boolean = false) {
        app = context.applicationContext
        current = manifest
        forceStop = stopAndInstall
        listenToDownloads()
        val p = prefs(context)
        p.edit().putLong(KEY_PENDING, manifest.versionCode).apply()
        if (!canInstall(context)) { set(context, UpdateState.Problem(ProblemKind.NotAllowed, manifest)); return }
        if (manifest.size <= 0) { set(context, UpdateState.Problem(ProblemKind.CheckFailed, manifest)); return }
        if (readyApk(context, manifest)) { afterDownload(context, manifest); return }
        set(context, UpdateState.Downloading)
        Downloads.engine(context, itemOf(manifest)).request()
    }

    /** The checked file is here: the install goes on at once while the app is in front, otherwise a notice carries it. */
    private fun afterDownload(context: Context, m: ChannelManifest) {
        set(context, UpdateState.Downloaded(m))
        if (!AppVisibility.visible && !forceStop) { UpdateNotices.downloaded(context, m); return }
        gateThenCommit(context, m)
    }

    /** The idle gate: nothing of HQ's is running (no chat turn, backup, outbox send or other download), or the owner chose to stop it. */
    private fun gateThenCommit(context: Context, m: ChannelManifest) {
        if (!forceStop) {
            val reason = busyNow(context)
            if (reason != null) { set(context, UpdateState.Busy(m, reason)); return }
        }
        forceStop = false
        val apk = File(Downloads.dirOf(context), APK_NAME)
        if (!fileMatches(apk, m)) {
            apk.delete()
            Downloads.get(ITEM_ID)?.discard()
            set(context, UpdateState.Problem(ProblemKind.NotInstalled, m))
            return
        }
        try {
            commit(context, apk)
            prefs(context).edit().putLong(KEY_INSTALLING, m.versionCode).putString(KEY_INSTALLING_NAME, m.version).remove(KEY_PENDING).apply()
            UpdateNotices.clear(context)
            set(context, UpdateState.Installing)
        } catch (e: IOException) {
            Log.w(TAG, "install failed: ${e.javaClass.simpleName}")
            set(context, UpdateState.Problem(ProblemKind.NotInstalled, m))
        } catch (e: SecurityException) {
            Log.w(TAG, "install refused: ${e.javaClass.simpleName}")
            set(context, UpdateState.Problem(ProblemKind.NotAllowed, m))
        }
    }

    private fun readyApk(context: Context, m: ChannelManifest): Boolean = m.size > 0 && File(Downloads.dirOf(context), APK_NAME).let { it.exists() && it.length() == m.size }

    private fun fileMatches(apk: File, m: ChannelManifest): Boolean {
        if (!apk.exists() || apk.length() != m.size) return false
        val digest = MessageDigest.getInstance("SHA-256")
        apk.inputStream().use { input ->
            val buffer = ByteArray(BUFFER_BYTES)
            while (true) { val n = input.read(buffer); if (n < 0) break; digest.update(buffer, 0, n) }
        }
        return digest.digest().joinToString("") { "%02x".format(it) } == m.sha256
    }

    /**
     * The owner chose "Wait until it's done": polls until HQ is free, then installs; [cancelWait] ends it ("Not now").
     * Blocking, so call it off the main thread.
     */
    fun waitThenInstall(context: Context, manifest: ChannelManifest) {
        waiting = true
        try {
            while (waiting) {
                val busy = busyStatus(context)
                val reason = busyNow(context, busy)
                if (reason == null) break
                set(context, UpdateState.Waiting(manifest, busy?.chatTurns ?: 0))
                try { Thread.sleep(UpdateLogic.BUSY_POLL_MS) } catch (_: InterruptedException) { waiting = false }
            }
            if (waiting) install(context, manifest, stopAndInstall = true) else set(context, UpdateState.Available(manifest))
        } finally {
            waiting = false
        }
    }

    fun cancelWait() { waiting = false }

    /** "Not now" on the busy dialog: the update stays offered, and the checked file stays too. */
    fun dismissBusy(manifest: ChannelManifest) {
        if (flow.value is UpdateState.Busy) set(app, if (readyApk(app ?: return, manifest)) UpdateState.Downloaded(manifest) else UpdateState.Available(manifest))
    }

    @Volatile private var waiting = false

    /** Why an install would stop work right now, or null. HQ not answering counts as free: there is nothing of its to stop. */
    fun busyNow(context: Context, status: BusyStatus? = busyStatus(context)): BusyReason? =
        UpdateLogic.busyReason(status, outboxWaiting(context), VoiceEngines.downloading() != null || otherDownload())

    private fun otherDownload(): Boolean = Downloads.states.value.any { (id, s) -> id != ITEM_ID && s is DlState.Downloading }

    private fun outboxWaiting(context: Context): Boolean = example.hq.spike.safe.NotesCopy.waitingCount(context.filesDir) > 0

    private fun busyStatus(context: Context): BusyStatus? = try {
        val token = HubService.token(context)
        val request = Request.Builder().url("http://127.0.0.1:$BUSY_PORT${UpdateLogic.BUSY_PATH}").header("Authorization", "******").build()
        http.newBuilder().connectTimeout(2, TimeUnit.SECONDS).readTimeout(3, TimeUnit.SECONDS).build().newCall(request).execute().use { r ->
            if (r.isSuccessful) UpdateLogic.parseBusy(r.body.string()) else null
        }
    } catch (_: IOException) {
        null
    }

    /** What Android's installer answered, from [InstallResultReceiver]. Success needs no state: the app is replaced. */
    internal fun finished(status: Int, message: String?) {
        val kind = UpdateLogic.problemOf(status, message) ?: return
        set(app, UpdateState.Problem(kind, current))
    }

    /** The confirm cannot open over another app: the checked update is shown again, and its notice asks to install. */
    internal fun confirmLater(context: Context) {
        app = context.applicationContext
        val m = current ?: return
        set(context, UpdateState.Downloaded(m))
        UpdateNotices.downloaded(context, m)
    }

    /** The first thing after a restart: an install that went through is told once, and what it left behind is removed. */
    private fun settleInstalled(context: Context) {
        val p = prefs(context)
        val code = p.getLong(KEY_INSTALLING, 0)
        if (code <= 0 || installedCode(context) < code) return
        val name = p.getString(KEY_INSTALLING_NAME, null) ?: installedName(context)
        p.edit().remove(KEY_INSTALLING).remove(KEY_INSTALLING_NAME).remove(KEY_PENDING).putLong(KEY_INSTALLED_AT, System.currentTimeMillis()).putString(KEY_INSTALLED_NAME, name).apply()
        dropSaved(context)
        File(Downloads.dirOf(context), APK_NAME).delete()
        UpdateNotices.installed(context, name)
        lineFlow.value = Line(R.string.upd_updated_snack, name)
    }

    /** A checked manifest saved on disk, signed again, offers its update without the network. */
    private fun reloadSaved(context: Context) {
        val dir = savedDir(context)
        val manifestFile = File(dir, UpdateLogic.CHANNEL_FILE)
        val sigFile = File(dir, UpdateLogic.CHANNEL_FILE + ".minisig")
        if (!manifestFile.exists() || !sigFile.exists()) return
        if (!signatureHolds(context, manifestFile, sigFile)) { dropSaved(context); return }
        val manifest = UpdateLogic.parse(manifestFile.readText()) ?: run { dropSaved(context); return }
        val verdict = UpdateLogic.verdict(manifest, installedCode(context), Build.VERSION.SDK_INT, bucket(context), channel(context) == UpdateChannel.Main, true)
        when (verdict) {
            is UpdateVerdict.Available -> { current = verdict.manifest; set(context, if (readyApk(context, manifest)) UpdateState.Downloaded(manifest) else UpdateState.Available(manifest)) }
            is UpdateVerdict.NeedsNewerAndroid -> set(context, UpdateState.NeedsNewerAndroid(verdict.minSdk, verdict.version))
            is UpdateVerdict.InstalledWithdrawn -> set(context, UpdateState.InstalledWithdrawn(installedName(context), verdict.fix))
            else -> dropSaved(context)
        }
        val engine = Downloads.get(ITEM_ID)
        if (engine != null && current != null && flow.value is UpdateState.Available) onDownload(engine.state.value)
    }

    private fun savedDir(context: Context) = File(context.noBackupFilesDir, DIR).apply { mkdirs() }

    private fun dropSaved(context: Context) {
        File(savedDir(context), UpdateLogic.CHANNEL_FILE).delete()
        File(savedDir(context), UpdateLogic.CHANNEL_FILE + ".minisig").delete()
    }

    /** The Update page opened: the dot goes, an old check is renewed, and a checked file waiting for its install goes on. */
    fun pageOpened(context: Context) {
        app = context.applicationContext
        val m = manifestOf(flow.value) ?: current
        if (m != null) prefs(context).edit().putLong(KEY_OPENED, m.versionCode).apply()
        refreshDot()
        val s = flow.value
        if (s is UpdateState.Downloaded && canInstall(context)) { current = s.manifest; gateThenCommit(context, s.manifest); return }
        if (System.currentTimeMillis() - prefs(context).getLong(KEY_LAST_CHECK, 0) > PAGE_CHECK_MS) check(context)
    }

    /** "Not now": the dot and the card stay away for seven days, or until a newer version. */
    fun notNow(context: Context) {
        val m = manifestOf(flow.value) ?: current ?: return
        prefs(context).edit().putLong(KEY_SNOOZE_CODE, m.versionCode).putLong(KEY_SNOOZE_UNTIL, System.currentTimeMillis() + SNOOZE_MS).apply()
        refreshDot()
    }

    /** Coming back to the app: an install permission given in Android's settings starts the saved download by itself. */
    fun onForeground(context: Context) {
        app = context.applicationContext
        val s = flow.value
        val m = (s as? UpdateState.Problem)?.takeIf { it.kind == ProblemKind.NotAllowed }?.manifest ?: return
        if (prefs(context).getLong(KEY_PENDING, 0) != m.versionCode || !canInstall(context)) return
        lineFlow.value = Line(R.string.upd_back_allowed)
        install(context, m)
    }

    fun facts(context: Context): Facts {
        val p = prefs(context)
        val at = p.getLong(KEY_INSTALLED_AT, 0)
        val today = if (at > 0 && java.time.LocalDate.now() == java.time.Instant.ofEpochMilli(at).atZone(java.time.ZoneId.systemDefault()).toLocalDate()) p.getString(KEY_INSTALLED_NAME, null) else null
        return Facts(installedName(context), installedCode(context), Build.VERSION.RELEASE.orEmpty(), p.getLong(KEY_LAST_CHECK, 0), today, canInstall(context))
    }

    fun manifestOf(s: UpdateState): ChannelManifest? = when (s) {
        is UpdateState.Available -> s.manifest
        is UpdateState.Downloaded -> s.manifest
        is UpdateState.Busy -> s.manifest
        is UpdateState.Waiting -> s.manifest
        is UpdateState.Problem -> s.manifest
        UpdateState.Downloading, UpdateState.Installing -> current
        else -> null
    }

    private fun set(context: Context?, s: UpdateState) {
        if (context != null) app = context.applicationContext
        flow.value = s
        refreshDot()
    }

    private fun refreshDot() {
        val context = app ?: return
        val m = manifestOf(flow.value)
        val p = prefs(context)
        val now = System.currentTimeMillis()
        val snoozed = m != null && p.getLong(KEY_SNOOZE_CODE, 0) == m.versionCode && now < p.getLong(KEY_SNOOZE_UNTIL, 0)
        hiddenFlow.value = snoozed
        dotFlow.value = m != null && !snoozed && p.getLong(KEY_OPENED, 0) != m.versionCode && flow.value !is UpdateState.Installing
    }

    private fun canInstall(context: Context): Boolean = context.packageManager.canRequestPackageInstalls()

    private fun installedName(context: Context): String =
        context.packageManager.getPackageInfo(context.packageName, 0).versionName.orEmpty().ifBlank { installedCode(context).toString() }

    /** The percent bucket this phone is in for staged rollouts: drawn once, kept. */
    private fun bucket(context: Context): Int {
        val p = prefs(context)
        if (!p.contains(KEY_BUCKET)) p.edit().putInt(KEY_BUCKET, java.util.concurrent.ThreadLocalRandom.current().nextInt(BUCKETS)).apply()
        return p.getInt(KEY_BUCKET, 0)
    }

    /** Null when the manifest is older than one already seen: the state stays as it was. A good one is saved, with its signature, for the next start. */
    private fun fetchVerdict(context: Context): UpdateVerdict? {
        val url = UpdateLogic.manifestUrl(channel(context))
        val staging = File(savedDir(context), "staging").apply { deleteRecursively(); mkdirs() }
        // The signature names the file it signed, so the manifest is saved under its release name.
        val manifestFile = File(staging, UpdateLogic.CHANNEL_FILE)
        val sigFile = File(staging, UpdateLogic.CHANNEL_FILE + ".minisig")
        try {
            manifestFile.writeBytes(getCapped(context, url))
            sigFile.writeBytes(getCapped(context, "$url.minisig"))
            if (!signatureHolds(context, manifestFile, sigFile)) throw UpdateException("The update information is not signed by the AgentHQ release key. Nothing was installed.")
            val manifest = UpdateLogic.parse(manifestFile.readText()) ?: throw UpdateException("The update information is not in a form this version understands.")
            val p = prefs(context)
            if (!UpdateLogic.seqOk(manifest, p.getLong(KEY_SEQ, 0))) return null
            p.edit().putLong(KEY_SEQ, manifest.seq).putBoolean(KEY_SAVE_COPY, manifest.switches.saveCopy).apply()
            val offered = p.getLong(KEY_OFFERED, 0) == manifest.versionCode
            val verdict = UpdateLogic.verdict(manifest, installedCode(context), Build.VERSION.SDK_INT, bucket(context), channel(context) == UpdateChannel.Main, offered)
            if (verdict is UpdateVerdict.Available || verdict is UpdateVerdict.NeedsNewerAndroid || verdict is UpdateVerdict.InstalledWithdrawn) {
                val dir = savedDir(context)
                manifestFile.copyTo(File(dir, UpdateLogic.CHANNEL_FILE), overwrite = true)
                sigFile.copyTo(File(dir, UpdateLogic.CHANNEL_FILE + ".minisig"), overwrite = true)
            }
            return verdict
        } finally {
            staging.deleteRecursively()
        }
    }

    private fun installedCode(context: Context): Long {
        val info = context.packageManager.getPackageInfo(context.packageName, 0)
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) info.longVersionCode else @Suppress("DEPRECATION") info.versionCode.toLong()
    }

    private fun getCapped(context: Context, url: String): ByteArray {
        http.newCall(Request.Builder().url(url).build()).execute().use { response ->
            if (!response.isSuccessful) throw UpdateException("The update server answered ${response.code}.")
            val source = response.body.source()
            if (source.request(UpdateLogic.MANIFEST_MAX_BYTES + 1)) throw UpdateException("The update information is too large.")
            return source.readByteArray().also { example.hq.spike.measure.Counters.get(context).addBytes(DataPurpose.AppUpdates, example.hq.spike.measure.Counters.metered(context), it.size.toLong()) }
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

    /** The signed manifest's `saveCopy` switch as last seen; on until a verified manifest says otherwise. */
    fun saveCopyAllowed(context: Context): Boolean = prefs(context).getBoolean(KEY_SAVE_COPY, true)

    private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private class UpdateException(message: String) : Exception(message)
}
