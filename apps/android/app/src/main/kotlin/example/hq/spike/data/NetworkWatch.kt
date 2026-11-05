package example.hq.spike.data

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * Whether the phone has a validated network, from the default network callback. [offlineSince] is when it last lost one, or null
 * while it has one; the 3 s wait before "You're offline" is in [ConnectionRules], so a blip never shows.
 */
class NetworkWatch(context: Context) {
    private val _offlineSince = MutableStateFlow<Long?>(null)
    val offlineSince: StateFlow<Long?> = _offlineSince

    init {
        val cm = context.getSystemService(ConnectivityManager::class.java)
        if (cm != null) {
            _offlineSince.value = if (validated(cm.getNetworkCapabilities(cm.activeNetwork))) null else System.currentTimeMillis()
            try {
                cm.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
                    override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) { set(validated(caps)) }
                    override fun onLost(network: Network) { set(false) }
                })
            } catch (e: SecurityException) {
                _offlineSince.value = null
            }
        }
    }

    private fun set(online: Boolean) {
        if (online) _offlineSince.value = null else if (_offlineSince.value == null) _offlineSince.value = System.currentTimeMillis()
    }

    private fun validated(caps: NetworkCapabilities?) = caps != null && caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
}
