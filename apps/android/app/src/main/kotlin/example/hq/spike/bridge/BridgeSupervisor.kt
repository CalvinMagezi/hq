package example.hq.spike.bridge

import android.content.Context
import android.os.Build
import android.util.Log
import java.io.BufferedReader
import java.io.File
import java.io.InputStreamReader
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/**
 * Runs `hq bridge run` as a child of the app and keeps it running while the owner has paired devices
 * allowed in. It lives inside the hub's foreground service ([example.hq.spike.HubService]) so the
 * bridge connection stays up with the app in the background. Whatever later carries the edge stream
 * (workstream W1) is another protocol on the same child and needs no new service.
 *
 * Off by default. The child binds a UDP socket and accepts paired devices, so it runs only after the
 * owner turned "Paired devices" on (pairing a first device does that), and stops when they turn it off.
 */
object BridgeSupervisor {
    private const val TAG = "HQBRIDGE"
    private const val PREFS = "bridge"
    private const val KEY_ENABLED = "enabled"
    private const val RELAY_FILE = "bridge_relay"
    private const val DEFAULT_RELAY = "default"
    private const val BACKOFF_MIN_MS = 2_000L
    private const val BACKOFF_MAX_MS = 60_000L
    private const val STABLE_RUN_MS = 60_000L
    private const val QUIT_WAIT_MS = 3_000L
    private const val MAX_LOG_BYTES = 1L shl 20
    private const val NAME_MAX_CHARS = 40

    private val flow = MutableStateFlow(BridgeState())
    val state: StateFlow<BridgeState> = flow.asStateFlow()

    private val lock = Any()
    private val generation = AtomicInteger()
    private val writes = Executors.newSingleThreadExecutor { r -> Thread(r, "bridge-writer").apply { isDaemon = true } }

    @Volatile private var process: Process? = null
    /** Off only for a measurement (see PairingActivity): the bridge then never hears of a network change. */
    @Volatile var watchNetwork = true
    private var appContext: Context? = null

    fun isEnabled(context: Context): Boolean = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(KEY_ENABLED, false)

    /** Turns the bridge on or off for good (it is remembered) and starts or stops the child now. */
    fun setEnabled(context: Context, on: Boolean) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(KEY_ENABLED, on).apply()
        flow.update { it.copy(enabled = on) }
        if (on) start(context) else stop()
    }

    /** Called when the service starts. Does nothing while the owner has not turned the bridge on. */
    fun start(context: Context) {
        val app = context.applicationContext
        val enabled = isEnabled(app)
        flow.update { it.copy(enabled = enabled) }
        if (!enabled) return
        synchronized(lock) {
            if (appContext != null) return
            appContext = app
            val gen = generation.incrementAndGet()
            Thread({ supervise(app, gen) }, "bridge-supervisor").apply { isDaemon = true }.start()
        }
    }

    /** Called when the service ends or the owner turns the bridge off. Does not wait for the child: the supervisor thread ends it. */
    fun stop() {
        synchronized(lock) {
            if (appContext == null) return
            appContext = null
            generation.incrementAndGet()
        }
        flow.update(BridgeReducer::stopped)
        send(BridgeWire.quit())
    }

    /** Sends one command line. False when the child is not running. */
    fun send(line: String): Boolean {
        val child = process ?: return false
        writes.execute {
            runCatching {
                child.outputStream.write((line + "\n").toByteArray(Charsets.UTF_8))
                child.outputStream.flush()
            }.onFailure { Log.w(TAG, "could not write to the bridge: ${it.javaClass.simpleName}") }
        }
        return true
    }

    fun openTicket(): Boolean = send(BridgeWire.open())
    fun cancelTicket(): Boolean = send(BridgeWire.cancel())
    fun submitDigits(typed: String): Boolean = send(BridgeWire.digits(typed))
    fun join(link: String): Boolean {
        flow.update { it.copy(join = JoinState.Connecting) }
        return send(BridgeWire.join(link))
    }
    fun refreshPeers(): Boolean = send(BridgeWire.peers())
    fun revoke(id: String): Boolean = send(BridgeWire.revoke(id))
    fun ping(id: String): Boolean = send(BridgeWire.ping(id))
    fun networkChanged(): Boolean = send(BridgeWire.networkChange())
    fun forgetJoin() = flow.update(BridgeReducer::resetJoin)
    fun forgetTicketEnd() = flow.update { it.copy(ticketEnd = null) }
    fun dismissLine() = flow.update { it.copy(line = null) }

    private fun supervise(context: Context, gen: Int) {
        var backoff = BACKOFF_MIN_MS
        while (gen == generation.get()) {
            val began = System.currentTimeMillis()
            runOnce(context, gen)
            if (gen != generation.get()) return
            val ran = System.currentTimeMillis() - began
            backoff = if (ran >= STABLE_RUN_MS) BACKOFF_MIN_MS else minOf(backoff * 2, BACKOFF_MAX_MS)
            Log.w(TAG, "bridge ended after ${ran / 1000} s, starting again in ${backoff / 1000} s")
            try { Thread.sleep(backoff) } catch (e: InterruptedException) { return }
        }
    }

    private fun runOnce(context: Context, gen: Int) {
        val child = try { launch(context) } catch (e: Exception) {
            Log.e(TAG, "could not start the bridge: ${e.javaClass.simpleName}")
            return
        }
        process = child
        val net = NetworkWatcher(context) {
            Log.i(TAG, if (watchNetwork) "network changed, telling the bridge" else "network changed, not telling the bridge (measurement)")
            if (watchNetwork) networkChanged()
        }
        try {
            net.start()
            BufferedReader(InputStreamReader(child.inputStream, Charsets.UTF_8)).use { reader ->
                while (gen == generation.get()) {
                    val line = reader.readLine() ?: break
                    handle(line)
                }
            }
        } finally {
            net.stop()
            if (process === child) process = null
            flow.update(BridgeReducer::stopped)
            endChild(child)
        }
    }

    private fun handle(line: String) {
        val event = BridgeWire.parse(line) ?: return
        // Names of events only: a ticket's link holds the one-time code and the digits are secret to the person.
        Log.i(TAG, "bridge: ${event.javaClass.simpleName}")
        flow.update { BridgeReducer.reduce(it, event, System.currentTimeMillis()) }
        if (event is BridgeEvent.Ready) refreshPeers()
        if (event is BridgeEvent.Joined || event is BridgeEvent.Decision) refreshPeers()
    }

    private fun launch(context: Context): Process {
        val lib = File(context.applicationInfo.nativeLibraryDir, "libhq.so")
        val home = File(context.filesDir, "hqhome").apply { mkdirs() }
        val relay = relay(context)
        // With a relay, a ticket needs no addresses: the code is much smaller and shows no IP address.
        val cmd = listOf(lib.path, "bridge", "run", "--relay", relay, "--name", deviceName()) + if (relay == "none") emptyList() else listOf("--no-addrs")
        val pb = ProcessBuilder(cmd)
        // No credentials: the bridge needs the vault database and its own key, nothing else.
        pb.environment().apply {
            clear()
            put("HOME", home.path)
            put("TMPDIR", context.cacheDir.path)
            put("HQ_PROFILE", "mobile")
            put("HQ_VAULT_PATH", File(home, ".vault").path)
            put("HQ_CONFIG_PATH", File(home, "config.yaml").path)
            // Name lookups use the system resolver; the library path is what a bionic executable needs.
            System.getenv("ANDROID_ROOT")?.let { put("ANDROID_ROOT", it) }
            System.getenv("ANDROID_DATA")?.let { put("ANDROID_DATA", it) }
        }
        pb.redirectError(ProcessBuilder.Redirect.appendTo(trimmed(File(context.filesDir, "bridge.log"))))
        return pb.start()
    }

    /** The relay to use: "none", "default" (n0's public relays, for development) or a relay URL, from files/bridge_relay. */
    internal fun relay(context: Context): String {
        val text = runCatching { File(context.filesDir, RELAY_FILE).takeIf { it.isFile && it.length() < MAX_RELAY_FILE_BYTES }?.readText()?.trim() }.getOrNull()
        return text?.takeIf { it.isNotEmpty() && it.none(Char::isWhitespace) } ?: DEFAULT_RELAY
    }

    private const val MAX_RELAY_FILE_BYTES = 512L

    private fun deviceName(): String = Build.MODEL.filter { it.isLetterOrDigit() || it == ' ' || it == '-' }.trim().take(NAME_MAX_CHARS).ifEmpty { "Android phone" }

    private fun trimmed(log: File): File {
        if (log.length() > MAX_LOG_BYTES) log.delete()
        return log
    }

    private fun endChild(child: Process) {
        runCatching { child.outputStream.write((BridgeWire.quit() + "\n").toByteArray()); child.outputStream.flush() }
        runCatching { child.outputStream.close() }
        if (!child.waitFor(QUIT_WAIT_MS, TimeUnit.MILLISECONDS)) child.destroyForcibly()
    }
}
