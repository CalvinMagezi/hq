package example.hq.spike.bridge

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Handler
import android.os.Looper
import android.util.Log

/**
 * Tells the bridge when a physical network comes, goes or changes its addresses. Native code gets no
 * such news from the system, and without it iroh keeps sending on the old path until it times out (16 to
 * 22 s measured on a Wi-Fi to mobile data switch). A burst of callbacks (a network coming up reports itself
 * several times) becomes one call, [SETTLE_MS] after the last.
 *
 * It watches every internet network that is not a VPN, not the default one: a phone with a VPN has the
 * VPN as its default network, and a switch from Wi-Fi to mobile data under it never changes that, so the
 * default-network callback stayed silent on the test phone (the owner's split tunnel).
 */
class NetworkWatcher(context: Context, private val onChange: () -> Unit) {
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val fire = Runnable { onChange() }
    private var registered = false

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = changed()
        override fun onLost(network: Network) = changed()
        override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) = changed()
        // Not onCapabilitiesChanged: it reports every change of signal strength.
    }

    private fun changed() {
        main.removeCallbacks(fire)
        main.postDelayed(fire, SETTLE_MS)
    }

    fun start() {
        if (registered) return
        // Fires once at once for each network there is, which is a harmless first probe. Without the
        // watcher the bridge still works; it only learns of a network change late.
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            .build()
        try {
            manager.registerNetworkCallback(request, callback)
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
