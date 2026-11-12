package example.hq.spike.calendar

import android.util.Log
import example.hq.spike.data.ChatConnection
import example.hq.spike.data.ChatEvent
import example.hq.spike.data.HqClient
import example.hq.spike.data.HqUnavailable
import java.time.Instant
import java.time.format.DateTimeParseException
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.filterIsInstance
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** The calls the handler makes back to HQ. */
interface DeviceActionLink {
    suspend fun ack(actionId: String)
    suspend fun result(actionId: String, result: JSONObject)
    suspend fun pending(): List<ChatEvent.DeviceAction>
}

class HqDeviceActionLink(private val client: HqClient) : DeviceActionLink {
    override suspend fun ack(actionId: String) = client.ackDeviceAction(actionId)
    override suspend fun result(actionId: String, result: JSONObject) = client.postDeviceActionResult(actionId, result)
    override suspend fun pending() = client.pendingDeviceActions()
}

/**
 * Runs `calendar.create`, `calendar.list`, `calendar.update` and `calendar.delete` device actions. It lives on the process scope next to the chat socket, so an action is handled
 * whenever the socket is up, with or without a screen open. Only ids, phases and outcome codes are logged.
 */
class DeviceActionHandler(
    private val link: DeviceActionLink,
    private val creator: CalendarCreator,
    private val lister: CalendarLister,
    private val modifier: CalendarModifier,
    private val now: () -> Instant = Instant::now,
    private val log: (String) -> Unit = { Log.i(TAG, it) },
) {
    /** Action ids seen in this process, so a pending list and the socket never run one twice. */
    private val seen = ConcurrentHashMap.newKeySet<String>()

    /** Results the phone made but HQ has not taken yet, posted again on the next look at the pending list. */
    private val unposted = ConcurrentHashMap<String, JSONObject>()

    fun start(scope: CoroutineScope, connection: ChatConnection) {
        scope.launch { connection.events.filterIsInstance<ChatEvent.DeviceAction>().collect { launch { handle(it) } } }
        scope.launch {
            var wasUp = false
            connection.connected.collect { up ->
                if (up && !wasUp) settlePending()
                wasUp = up
            }
        }
    }

    /** Takes the open actions HQ still lists: posts a result already made, or runs one the socket missed. */
    suspend fun settlePending() {
        val open = try { link.pending() } catch (e: HqUnavailable) { return }
        open.forEach { handle(it) }
    }

    suspend fun handle(action: ChatEvent.DeviceAction) {
        if (action.name !in HANDLED || action.version != CalendarLimits.WIRE_VERSION) return
        unposted[action.actionId]?.let { post(action.actionId, it); return }
        if (!seen.add(action.actionId)) return
        if (expired(action)) { log("device_action ${action.actionId} phase=expired"); return }
        if (!ackSent(action.actionId)) return
        val outcome = withContext(Dispatchers.IO) { run(action) }
        log("device_action ${action.actionId} phase=done outcome=${outcome.code}")
        post(action.actionId, outcome.toJson())
    }

    private fun run(action: ChatEvent.DeviceAction): CalendarOutcome = when (action.name) {
        CalendarLimits.LIST_ACTION_NAME -> runList(action)
        CalendarModifyLimits.UPDATE_ACTION_NAME -> runModify(CalendarModifyRequests.parseUpdate(action.payload), modifier::update)
        CalendarModifyLimits.DELETE_ACTION_NAME -> runModify(CalendarModifyRequests.parseDelete(action.payload), modifier::delete)
        else -> runCreate(action)
    }

    private fun runModify(parsed: ParsedModify, act: (ModifyRequest) -> CalendarOutcome): CalendarOutcome = when (parsed) {
        is ParsedModify.Bad -> parsed.outcome
        is ParsedModify.Ok -> act(parsed.request)
    }

    private fun runCreate(action: ChatEvent.DeviceAction): CalendarOutcome = when (val parsed = CalendarRequests.parse(action.payload)) {
        is ParsedRequest.Bad -> parsed.outcome
        is ParsedRequest.Ok -> creator.create(parsed.request)
    }

    private fun runList(action: ChatEvent.DeviceAction): CalendarOutcome = when (val parsed = CalendarListRequests.parse(action.payload)) {
        is ParsedListRequest.Bad -> parsed.outcome
        is ParsedListRequest.Ok -> lister.list(parsed.request)
    }

    private fun expired(action: ChatEvent.DeviceAction): Boolean {
        val at = action.expiresAt ?: return false
        return try { !Instant.parse(at).isAfter(now()) } catch (e: DateTimeParseException) { false }
    }

    private suspend fun ackSent(actionId: String): Boolean = try {
        link.ack(actionId)
        true
    } catch (e: HqUnavailable) {
        // Not acked means HQ reports the phone unreachable and nothing was sent to the provider; a later pending look may retry.
        seen.remove(actionId)
        log("device_action $actionId phase=ack_failed")
        false
    }

    private suspend fun post(actionId: String, result: JSONObject) {
        try {
            link.result(actionId, result)
            unposted.remove(actionId)
        } catch (e: HqUnavailable) {
            unposted[actionId] = result
            log("device_action $actionId phase=result_failed")
        }
    }

    private companion object {
        const val TAG = "DeviceAction"
        val HANDLED = setOf(CalendarLimits.ACTION_NAME, CalendarLimits.LIST_ACTION_NAME, CalendarModifyLimits.UPDATE_ACTION_NAME, CalendarModifyLimits.DELETE_ACTION_NAME)
    }
}
