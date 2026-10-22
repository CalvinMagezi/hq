package example.hq.spike.bridge

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.os.Handler
import android.os.Looper
import android.util.Log

/**
 * Tells the bridge when Android's default network changes. Native code gets no such news from the
 * system, and without it iroh keeps sending on the old path until it times out (about 18 s measured on
 * a Wi-Fi to mobile data switch). A burst of callbacks (a network coming up reports itself several times)
 * becomes one call, [SETTLE_MS] after the last.
 */
class NetworkWatcher(context: Context, private val onChange: () -> Unit) {
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val fire = Runnable { onChange() }
    private var registered = false

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = changed()
        override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) = changed()
        // A lost network is followed by onAvailable for the next one; with none there is nothing to probe.
    }

    private fun changed() {
        main.removeCallbacks(fire)
        main.postDelayed(fire, SETTLE_MS)
    }

    fun start() {
        if (registered) return
        // Fires once at once with the current network, which is a harmless first probe. Without the
        // watcher the bridge still works; it only learns of a network change late.
        try {
            manager.registerDefaultNetworkCallback(callback)
            registered = true
        } catch (e: RuntimeException) {
            Log.w("HQBRIDGE", "network watcher not started: ${e.javaClass.simpleName}")
        }
    }

    fun stop() {
        if (!registered) return
        registered = false
        main.removeCallbacks(fire)
        runCatching { manager.unregisterNetworkCallback(callback) }
    }

    companion object {
        const val SETTLE_MS = 300L
    }
}
