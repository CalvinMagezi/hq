package example.hq.spike.ui.settings

import example.hq.spike.data.HqClient
import example.hq.spike.data.HqUnavailable
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

enum class ClearState { Idle, Busy, Failed, Done }

/** The clear's states as plain rules: one run at a time, and what each ending leaves. */
object ClearRules {
    fun begin(now: ClearState): ClearState? = if (now == ClearState.Busy) null else ClearState.Busy

    fun end(ok: Boolean): ClearState = if (ok) ClearState.Done else ClearState.Failed
}

/**
 * "Clear what HQ remembers" runs here, not in the page, so turning the phone does not cancel a request that is on its way;
 * the page reads [state] whenever it is on screen.
 */
object SuggestsClear {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val _state = MutableStateFlow(ClearState.Idle)
    val state: StateFlow<ClearState> = _state

    fun start(client: HqClient) {
        val next = ClearRules.begin(_state.value) ?: return
        _state.value = next
        scope.launch {
            _state.value = ClearRules.end(try { client.postJson("/api/offers/clear"); true } catch (e: HqUnavailable) { false })
        }
    }

    fun reset() {
        if (_state.value != ClearState.Busy) _state.value = ClearState.Idle
    }
}
