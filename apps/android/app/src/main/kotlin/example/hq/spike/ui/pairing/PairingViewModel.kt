package example.hq.spike.ui.pairing

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.bridge.BridgePeer
import example.hq.spike.bridge.BridgeState
import example.hq.spike.bridge.BridgeSupervisor
import example.hq.spike.data.HqServices
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class PairingScreen { Home, Show, Scan, Paste, Confirm, Joining }

data class PairingUi(
    val screen: PairingScreen = PairingScreen.Home,
    /** Digits typed for a device that waits to be paired. */
    val typed: String = "",
    /** The text of the paste field, or the link a confirmation screen asks about. */
    val link: String = "",
    val line: String? = null,
    val askRemove: BridgePeer? = null,
)

class PairingViewModel(app: Application) : AndroidViewModel(app) {
    val bridge: StateFlow<BridgeState> = BridgeSupervisor.state
    private val mutableUi = MutableStateFlow(PairingUi())
    val ui: StateFlow<PairingUi> = mutableUi.asStateFlow()

    private var wantsTicket = false
    private var ticketAsked = false
    private var pendingJoin: String? = null

    init {
        BridgeSupervisor.start(app)
        viewModelScope.launch { bridge.collect(::drive) }
    }

    /** Things the person asked for before the bridge was up happen as soon as it is. */
    private fun drive(b: BridgeState) {
        if (!b.running) {
            ticketAsked = false
            return
        }
        if (wantsTicket && !ticketAsked && b.ticket == null && b.ticketEnd == null) {
            ticketAsked = true
            BridgeSupervisor.openTicket()
        }
        pendingJoin?.let {
            pendingJoin = null
            BridgeSupervisor.join(it)
        }
    }

    private fun turnOn() {
        val app = getApplication<Application>()
        HqServices.ensureHubRunning(app)
        if (!BridgeSupervisor.isEnabled(app)) BridgeSupervisor.setEnabled(app, true)
    }

    fun setPairedDevices(on: Boolean) {
        val app = getApplication<Application>()
        if (on) turnOn() else BridgeSupervisor.setEnabled(app, false)
    }

    fun go(screen: PairingScreen) = mutableUi.update { it.copy(screen = screen, line = null, typed = "") }

    fun showCode() {
        turnOn()
        BridgeSupervisor.forgetTicketEnd()
        wantsTicket = true
        ticketAsked = false
        go(PairingScreen.Show)
    }

    fun typeDigits(text: String) = mutableUi.update { it.copy(typed = PairingWords.typed(text)) }

    fun submitDigits() {
        val typed = mutableUi.value.typed
        if (typed.length == PairingWords.DIGITS_LENGTH) BridgeSupervisor.submitDigits(typed)
        mutableUi.update { it.copy(typed = "") }
    }

    fun editLink(text: String) = mutableUi.update { it.copy(link = text.take(PairLink.MAX_CHARS), line = null) }

    /** A link from outside the app: it is shown for the person to confirm, never followed. */
    fun confirmLink(link: String) {
        val ok = PairLink.accept(link)
        mutableUi.update { if (ok != null) it.copy(screen = PairingScreen.Confirm, link = ok, line = null) else it.copy(screen = PairingScreen.Home, line = PairingWords.NOT_OURS) }
    }

    /** The text of a scan or a paste. A code the person chose to scan or paste in this app starts at once. */
    fun joinText(text: String?) {
        val link = PairLink.accept(text)
        if (link == null) {
            mutableUi.update { it.copy(line = PairingWords.NOT_OURS) }
            return
        }
        startJoin(link)
    }

    fun startJoin(link: String) {
        turnOn()
        BridgeSupervisor.forgetJoin()
        if (BridgeSupervisor.state.value.running) BridgeSupervisor.join(link) else pendingJoin = link
        mutableUi.update { it.copy(screen = PairingScreen.Joining, link = "", line = null) }
    }

    /** Leaves whatever screen is open for the home screen, dropping a ticket or a join in progress. */
    fun home() {
        if (wantsTicket) BridgeSupervisor.cancelTicket()
        wantsTicket = false
        ticketAsked = false
        pendingJoin = null
        BridgeSupervisor.forgetJoin()
        BridgeSupervisor.forgetTicketEnd()
        go(PairingScreen.Home)
        BridgeSupervisor.refreshPeers()
    }

    fun askRemove(peer: BridgePeer?) = mutableUi.update { it.copy(askRemove = peer) }

    fun confirmRemove() {
        mutableUi.value.askRemove?.let { BridgeSupervisor.revoke(it.id) }
        askRemove(null)
    }

    fun refresh() {
        BridgeSupervisor.refreshPeers()
    }

    fun dismissLine() {
        mutableUi.update { it.copy(line = null) }
        BridgeSupervisor.dismissLine()
    }

    override fun onCleared() {
        if (wantsTicket) BridgeSupervisor.cancelTicket()
    }
}
