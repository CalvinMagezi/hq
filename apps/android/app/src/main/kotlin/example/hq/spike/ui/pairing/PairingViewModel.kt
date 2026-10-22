package example.hq.spike.ui.pairing

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import example.hq.spike.bridge.BridgePeer
import example.hq.spike.bridge.BridgeState
import example.hq.spike.bridge.BridgeSupervisor
import example.hq.spike.bridge.JoinState
import example.hq.spike.bridge.TicketEnd
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
        // The bridge could not make a code: ask again only when the person does.
        if (b.ticketEnd == TicketEnd.Failed) wantsTicket = false
        if (wantsTicket && !ticketAsked && b.ticket == null && b.ticketEnd == null) {
            ticketAsked = true
            BridgeSupervisor.openTicket()
        }
        pendingJoin?.let {
            pendingJoin = null
            BridgeSupervisor.join(it)
        }
    }

    /** Something live that a link from another app must not replace: a code on show, a device waiting for digits, a join under way. */
    private fun pairingInProgress(): Boolean {
        val b = bridge.value
        return b.ticket != null || b.request != null || b.join.inFlight() || pendingJoin != null
    }

    private fun runForPairing() {
        val app = getApplication<Application>()
        HqServices.ensureHubRunning(app)
        BridgeSupervisor.startForPairing(app)
    }

    fun setPairedDevices(on: Boolean) {
        val app = getApplication<Application>()
        if (on) {
            HqServices.ensureHubRunning(app)
            BridgeSupervisor.setEnabled(app, true)
        } else {
            BridgeSupervisor.setEnabled(app, false)
        }
    }

    fun go(screen: PairingScreen) = mutableUi.update { it.copy(screen = screen, line = null, typed = "", link = "") }

    fun showCode() {
        wantsTicket = true
        ticketAsked = false
        runForPairing()
        BridgeSupervisor.forgetTicketEnd()
        go(PairingScreen.Show)
        // A state that does not change (the bridge already up, no end to forget) emits nothing for `drive` to hear.
        drive(bridge.value)
    }

    fun typeDigits(text: String) = mutableUi.update { it.copy(typed = PairingWords.typed(text)) }

    fun submitDigits() {
        val typed = mutableUi.value.typed
        if (typed.length == PairingWords.DIGITS_LENGTH) BridgeSupervisor.submitDigits(typed)
        mutableUi.update { it.copy(typed = "") }
    }

    fun editLink(text: String) = mutableUi.update { it.copy(link = text.take(PairLink.MAX_CHARS), line = null) }

    /** A link from outside the app: it is shown for the person to confirm, never followed. It does not replace a pairing in progress. */
    fun confirmLink(link: String) {
        if (pairingInProgress()) {
            mutableUi.update { it.copy(line = PairingWords.FINISH_FIRST) }
            return
        }
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
        // Only from where a code is taken: the scanner can read the same code twice before its screen is gone,
        // or once more after the person has left it.
        if (mutableUi.value.screen !in TAKES_A_CODE) return
        runForPairing()
        BridgeSupervisor.forgetJoin()
        if (BridgeSupervisor.state.value.running) BridgeSupervisor.join(link) else pendingJoin = link
        mutableUi.update { it.copy(screen = PairingScreen.Joining, link = "", line = null) }
    }

    /** Leaves whatever screen is open for the home screen, dropping a ticket or a join in progress. */
    fun home() {
        if (wantsTicket || bridge.value.join.inFlight()) BridgeSupervisor.cancelPairing()
        wantsTicket = false
        ticketAsked = false
        pendingJoin = null
        BridgeSupervisor.forgetJoin()
        BridgeSupervisor.forgetTicketEnd()
        go(PairingScreen.Home)
        // The bridge was started only for this pairing unless the owner has it on.
        BridgeSupervisor.stopUnlessEnabled(getApplication())
        BridgeSupervisor.refreshPeers()
    }

    /**
     * The screen went out of sight: a join that was waiting for the bridge to start must not fire later, with
     * nobody looking, and a bridge that was started only for a pairing that is not live goes off again.
     */
    fun onHidden() {
        pendingJoin = null
        val b = bridge.value
        if (b.ticket == null && b.request == null && !b.join.inFlight()) BridgeSupervisor.stopUnlessEnabled(getApplication())
    }

    fun askRemove(peer: BridgePeer?) = mutableUi.update { it.copy(askRemove = peer) }

    fun confirmRemove() {
        val peer = mutableUi.value.askRemove
        askRemove(null)
        if (peer != null && !BridgeSupervisor.revoke(peer.id)) mutableUi.update { it.copy(line = PairingWords.NOT_REMOVED) }
    }

    fun refresh() {
        BridgeSupervisor.refreshPeers()
    }

    fun dismissLine() {
        mutableUi.update { it.copy(line = null) }
        BridgeSupervisor.dismissLine()
    }

    override fun onCleared() {
        if (wantsTicket || bridge.value.join.inFlight()) BridgeSupervisor.cancelPairing()
        BridgeSupervisor.stopUnlessEnabled(getApplication())
    }

    private companion object {
        val TAKES_A_CODE = setOf(PairingScreen.Scan, PairingScreen.Paste, PairingScreen.Confirm)
    }

    private fun JoinState.inFlight() = this is JoinState.Connecting || this is JoinState.ShowDigits
}
