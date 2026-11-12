package example.hq.spike.contacts

import example.hq.spike.data.ChatEvent
import example.hq.spike.deviceactions.DeviceActionModule
import example.hq.spike.deviceactions.DeviceOutcome

class ContactsModule(private val searcher: ContactsSearcher) : DeviceActionModule {
    override val actions = setOf(ContactsLimits.ACTION_NAME)
    override val wireVersion = ContactsLimits.WIRE_VERSION

    override fun run(action: ChatEvent.DeviceAction): DeviceOutcome = when (val parsed = ContactsRequests.parse(action.payload)) {
        is ParsedContactsRequest.Bad -> parsed.outcome
        is ParsedContactsRequest.Ok -> searcher.search(parsed.request)
    }
}
