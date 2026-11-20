package example.hq.spike.ui.inbox

import androidx.annotation.StringRes
import example.hq.spike.R
import example.hq.spike.data.HqClient.InboxItem
import example.hq.spike.data.SandboxGrant

data class ParsedSandboxGrant(
    val sessionId: String?,
    val taskId: String?,
    val capability: String,
    val resource: String,
    val helperReason: String,
)

/** Parses and formats sandbox-grants inbox requests to match the web SandboxGrantCard wording. */
object SandboxGrantWords {
    private const val REASON_LIMIT = 300
    private val SESSION_ASK = Regex("""Helper session (\S+) asked to (.+?)\.\s+HQ could not""")
    private val HELPER_WORDS = Regex("""The helper's own words, not HQ's:\s*"([\s\S]*?)"\.\s*Dismiss to refuse\.?""")
    private val TITLE_PREFIX = Regex("""^A helper wants to\s+""", RegexOption.IGNORE_CASE)
    private val DESCRIBE_PATTERNS = listOf(
        Regex("^write to (.+)$") to "fs_write",
        Regex("^read (.+)$") to "fs_read",
        Regex("^connect to local port (.+)$") to "loopback",
        Regex("^reach (.+)$") to "egress",
    )

    fun parse(item: InboxItem, grants: List<SandboxGrant> = emptyList()): ParsedSandboxGrant {
        val desc = item.description
        val sessionMatch = SESSION_ASK.find(desc)
        val sessionId = sessionMatch?.groupValues?.get(1)?.ifBlank { null }
        val phrase = sessionMatch?.groupValues?.get(2) ?: item.title.trim().replace(TITLE_PREFIX, "")
        val (capability, resource) = parseDescribePhrase(phrase)
        val parsedReason = HELPER_WORDS.find(desc)?.groupValues?.get(1).orEmpty()

        val matched = grants.firstOrNull { g ->
            g.isResource && g.isPending &&
                (sessionId == null || g.sessionId == sessionId) &&
                (resource.isEmpty() || g.resource == resource)
        }

        return ParsedSandboxGrant(
            sessionId = matched?.sessionId ?: sessionId,
            taskId = matched?.taskId,
            capability = matched?.capability ?: capability,
            resource = matched?.resource ?: resource,
            helperReason = matched?.helperReason ?: parsedReason,
        )
    }

    /** Enriches an already-parsed card with matching pending grant details (such as task_id) from GET /api/sandbox-grants. */
    fun enrich(grant: ParsedSandboxGrant, grants: List<SandboxGrant>): ParsedSandboxGrant {
        val matched = grants.firstOrNull { g ->
            g.isResource && g.isPending &&
                (grant.sessionId == null || g.sessionId == grant.sessionId) &&
                (grant.resource.isEmpty() || g.resource == grant.resource)
        } ?: return grant
        return grant.copy(
            sessionId = matched.sessionId,
            taskId = matched.taskId ?: grant.taskId,
            capability = matched.capability.ifBlank { grant.capability },
            resource = matched.resource.ifBlank { grant.resource },
            helperReason = matched.helperReason.ifBlank { grant.helperReason },
        )
    }

    fun clipReason(reason: String): String {
        val trimmed = reason.trim()
        return if (trimmed.length > REASON_LIMIT) trimmed.take(REASON_LIMIT) + "..." else trimmed
    }

    @StringRes
    fun modeRes(capability: String): Int = when (capability) {
        "fs_write" -> R.string.grants_mode_fs_write
        "fs_read" -> R.string.grants_mode_fs_read
        "loopback" -> R.string.grants_mode_loopback
        "egress" -> R.string.grants_mode_egress
        else -> R.string.grants_mode_other
    }

    @StringRes
    fun stateRes(state: String): Int = when (state) {
        "active" -> R.string.grants_state_active
        "pending" -> R.string.grants_state_pending
        "revoked" -> R.string.grants_state_revoked
        "expired" -> R.string.grants_state_expired
        "refused" -> R.string.grants_state_refused
        "declined" -> R.string.grants_state_declined
        else -> R.string.grants_state_revoked
    }

    private fun parseDescribePhrase(phrase: String): Pair<String, String> {
        val clean = phrase.trim().removeSuffix(".")
        for ((pattern, cap) in DESCRIBE_PATTERNS) {
            val match = pattern.matchEntire(clean)
            if (match != null) return cap to match.groupValues[1].trim()
        }
        return "other" to clean
    }
}
