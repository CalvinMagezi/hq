package example.hq.spike.ui.settings

/** Tells when hq has started again after a restart was asked for: its start time changed, or it was seen down and is answering again. */
class RestartWatch(private val before: String?) {
    private var sawDown = false

    /** [answered] is whether hq replied to this poll; [stamp] is the start time it reported, if any. */
    fun back(answered: Boolean, stamp: String?): Boolean {
        if (!answered) { sawDown = true; return false }
        return sawDown || (stamp != null && stamp != before)
    }

    companion object {
        const val POLL_MS = 2_000L
        const val TIMEOUT_MS = 120_000L
    }
}
