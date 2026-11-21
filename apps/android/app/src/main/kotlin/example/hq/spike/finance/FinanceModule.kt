package example.hq.spike.finance

import example.hq.spike.data.ChatEvent
import example.hq.spike.deviceactions.DeviceActionModule
import example.hq.spike.deviceactions.DeviceOutcome

class FinanceModule(private val importer: FinanceImporter) : DeviceActionModule {
    override val actions = setOf(FinanceLimits.ACTION_NAME)
    override val wireVersion = FinanceLimits.WIRE_VERSION

    override fun run(action: ChatEvent.DeviceAction): DeviceOutcome = when (val parsed = FinanceRequests.parse(action.payload)) {
        is ParsedFinanceRequest.Bad -> parsed.outcome
        is ParsedFinanceRequest.Ok -> importer.import(action.actionId, parsed.request)
    }
}
