package example.hq.spike.deviceactions

import example.hq.spike.data.ChatEvent
import org.json.JSONObject

/** What a module reports back to HQ; the JSON is the domain's wire contract and passes through unchanged. */
interface DeviceOutcome {
    val code: String
    fun toJson(): JSONObject
}

/** request_id to the provider's id for what was made, kept in app storage so a retry never acts twice. Written synchronously. */
interface RequestLedger {
    fun get(requestId: String): Long?
    fun put(requestId: String, eventId: Long)
}

/** One domain (calendar, later contacts and the like) behind the generic handler. */
interface DeviceActionModule {
    val actions: Set<String>
    val wireVersion: Int
    fun run(action: ChatEvent.DeviceAction): DeviceOutcome
}

/** Action name to module. Two modules claiming one name is a wiring bug, so it fails at construction. */
class DeviceActionRegistry(modules: List<DeviceActionModule>) {
    private val byAction: Map<String, DeviceActionModule>

    init {
        val map = HashMap<String, DeviceActionModule>()
        modules.forEach { module ->
            module.actions.forEach { name ->
                require(map.put(name, module) == null) { "device action $name is claimed by two modules" }
            }
        }
        byAction = map
    }

    fun moduleFor(name: String): DeviceActionModule? = byAction[name]
}
