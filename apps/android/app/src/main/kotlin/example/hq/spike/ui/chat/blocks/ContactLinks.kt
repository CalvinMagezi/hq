package example.hq.spike.ui.chat.blocks

/** A contact link HQ built from digits: which contact row to show. */
data class ContactLink(val contactId: Long)

/** Reads `hq://contacts/contact/{id}`, and nothing looser: the link only navigates. */
object ContactLinks {
    private val SHAPE = Regex("hq://contacts/contact/(\\d{1,18})")

    fun parse(href: String): ContactLink? = SHAPE.matchEntire(href)?.let { ContactLink(it.groupValues[1].toLong()) }
}
