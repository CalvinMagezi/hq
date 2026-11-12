package example.hq.spike.calendar

import example.hq.spike.data.ChatEvent
import example.hq.spike.deviceactions.DeviceActionModule
import example.hq.spike.deviceactions.DeviceOutcome

class CalendarModule(
    private val creator: CalendarCreator,
    private val lister: CalendarLister,
    private val modifier: CalendarModifier,
) : DeviceActionModule {
    override val actions = setOf(CalendarLimits.ACTION_NAME, CalendarLimits.LIST_ACTION_NAME, CalendarModifyLimits.UPDATE_ACTION_NAME, CalendarModifyLimits.DELETE_ACTION_NAME)
    override val wireVersion = CalendarLimits.WIRE_VERSION

    override fun run(action: ChatEvent.DeviceAction): DeviceOutcome = when (action.name) {
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
}
