package example.hq.spike.download

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.storage.StorageManager
import android.telephony.TelephonyManager
import example.hq.spike.measure.Counters
import example.hq.spike.measure.DataPurpose
import example.hq.spike.ui.shell.Undo
import java.io.File
import java.io.IOException
import java.util.concurrent.TimeUnit
import okhttp3.OkHttpClient
import okhttp3.Request

/** The engine's view of this phone: the connection, free space, the network and the counters. */
class AndroidDownloadEnv(private val context: Context) : DownloadEnv {
    private val app = context.applicationContext
    private val connectivity = app.getSystemService(ConnectivityManager::class.java)
    private val answers = PaidWifi(PrefsAnswers(app))
    private val handler = Handler(Looper.getMainLooper())
    private val http = OkHttpClient.Builder().connectTimeout(15, TimeUnit.SECONDS).readTimeout(30, TimeUnit.SECONDS).build()

    override fun now() = System.currentTimeMillis()

    override fun net(): NetKind {
        val network = connectivity?.activeNetwork ?: return NetKind.None
        val caps = connectivity.getNetworkCapabilities(network) ?: return NetKind.None
        val online = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        val cellular = caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
        val wifiLike = caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) || caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
        val metered = !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
        val roaming = cellular && roaming(caps)
        val answer = if (wifiLike) answers.answer(fingerprint(network)) else null
        return DownloadRules.classify(online, cellular, wifiLike, metered, roaming, answer)
    }

    private fun roaming(caps: NetworkCapabilities): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_ROAMING)
        else app.getSystemService(TelephonyManager::class.java)?.isNetworkRoaming == true

    /** Gateway, DNS servers and search domain: what Android gives freely to tell one Wi-Fi from another. */
    private fun fingerprint(network: Network): String? {
        val link: LinkProperties = connectivity?.getLinkProperties(network) ?: return null
        val gateway = link.routes.firstOrNull { it.isDefaultRoute && it.gateway != null }?.gateway?.hostAddress
        return PaidWifi.fingerprint(gateway, link.dnsServers.mapNotNull { it.hostAddress }, link.domains)
    }

    override fun askAnyNetwork(): Boolean = Downloads.askAnyNetwork(app)

    override fun freeBytes(): Long = try {
        val manager = app.getSystemService(StorageManager::class.java)
        val dir = app.noBackupFilesDir
        manager.getAllocatableBytes(manager.getUuidForPath(dir))
    } catch (_: IOException) {
        app.noBackupFilesDir.usableSpace
    }

    override fun sleep(ms: Long) = Thread.sleep(ms)

    override fun undoMs(): Long = Undo.millis(app)

    override fun open(url: String, from: Long, ifRange: String?): Opened {
        val b = Request.Builder().url(url)
        if (from > 0) {
            b.header("Range", "bytes=$from-")
            if (ifRange != null) b.header("If-Range", ifRange)
        }
        val response = http.newCall(b.build()).execute()
        val start = Regex("bytes (\\d+)-").find(response.header("Content-Range").orEmpty())?.groupValues?.get(1)?.toLongOrNull() ?: 0L
        return Opened(response.code, start, response.header("ETag"), if (response.isSuccessful) response.body.byteStream() else null) { response.close() }
    }

    override fun count(purpose: DataPurpose, paid: Boolean, bytes: Long) = Counters.get(app).addBytes(purpose, paid, bytes)

    override fun rememberWifi(paid: Boolean) {
        val network = connectivity?.activeNetwork ?: return
        answers.remember(fingerprint(network), paid)
    }

    override fun later(delayMs: Long, run: () -> Unit) {
        handler.postDelayed({ Thread(run, "download-later").start() }, delayMs)
    }

    override fun spawn(run: () -> Unit) {
        Thread(run, "download").start()
    }

    /** True while the phone has a connection and a callback is wanted for changes to it. */
    fun watch(onChange: () -> Unit): ConnectivityManager.NetworkCallback? {
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = onChange()
            override fun onLost(network: Network) = onChange()
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) = onChange()
        }
        connectivity?.registerDefaultNetworkCallback(cb) ?: return null
        return cb
    }

    companion object {
        fun dir(context: Context): File = File(context.applicationContext.noBackupFilesDir, "downloads").apply { mkdirs() }
    }
}
