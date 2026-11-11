package example.hq.spike.ui.chat.blocks

/** An event link HQ built from numbers: which row, and which occurrence of it. */
data class CalendarLink(val eventId: Long, val beginMs: Long, val endMs: Long)

/** Reads `hq://calendar/event/{id}?begin={ms}&end={ms}`, and nothing looser: the link only navigates. */
object CalendarLinks {
    private val SHAPE = Regex("hq://calendar/event/(\\d{1,18})\\?begin=(\\d{1,18})&end=(\\d{1,18})")

    fun parse(href: String): CalendarLink? {
        val m = SHAPE.matchEntire(href) ?: return null
        val (id, begin, end) = m.destructured
        return CalendarLink(id.toLong(), begin.toLong(), end.toLong())
    }
}
