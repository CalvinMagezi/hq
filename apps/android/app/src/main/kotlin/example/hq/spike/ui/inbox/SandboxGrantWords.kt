package example.hq.spike.ui.inbox

import androidx.annotation.StringRes
import example.hq.spike.R
import example.hq.spike.data.HqClient.InboxItem
import example.hq.spike.data.REPO_SCOPE_PREFIX
import example.hq.spike.data.SandboxGrant
import example.hq.spike.data.SandboxGrantProfile
import example.hq.spike.data.SandboxSummaryResponse
import example.hq.spike.data.TASK_SCOPE_PREFIX

data class ParsedSandboxGrant(
    val sessionId: String?,
    val taskId: String?,
    val repo: String = "",
    val capability: String,
    val resource: String,
    val helperReason: String,
    val suggestedStarter: String? = null,
)

/** Owner choice for a sandbox access request, matching POST /api/notifications/{id}/action. */
sealed class SandboxGrantAnswer(val action: String, val starter: String? = null) {
    data object AllowOnce : SandboxGrantAnswer("approve")
    data object AllowTask : SandboxGrantAnswer("allow_task")
    data object AllowRepo : SandboxGrantAnswer("allow_repo")
    data class AllowStarter(val name: String) : SandboxGrantAnswer("allow_starter", name)
    data object Decline : SandboxGrantAnswer("dismiss")

    val isAllow: Boolean get() = this != Decline
}

/** Parses and formats sandbox-grants inbox requests, standing rules and summaries to match web wording. */
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
            g.isResource && !g.isStandingRule && g.isPending &&
                (sessionId == null || g.sessionId == sessionId) &&
                (resource.isEmpty() || g.resource == resource)
        }

        return ParsedSandboxGrant(
            sessionId = matched?.sessionId ?: sessionId,
            taskId = matched?.taskId,
            repo = matched?.repo.orEmpty(),
            capability = matched?.capability ?: capability,
            resource = matched?.resource ?: resource,
            helperReason = matched?.helperReason ?: parsedReason,
            suggestedStarter = matched?.suggestedStarter,
        )
    }

    /** Enriches an already-parsed card with matching pending grant details (task_id, repo, suggested_starter). */
    fun enrich(grant: ParsedSandboxGrant, grants: List<SandboxGrant>): ParsedSandboxGrant {
        val matched = grants.firstOrNull { g ->
            g.isResource && !g.isStandingRule && g.isPending &&
                (grant.sessionId == null || g.sessionId == grant.sessionId) &&
                (grant.resource.isEmpty() || g.resource == grant.resource)
        } ?: return grant
        return grant.copy(
            sessionId = matched.sessionId,
            taskId = matched.taskId ?: grant.taskId,
            repo = matched.repo.ifBlank { grant.repo },
            capability = matched.capability.ifBlank { grant.capability },
            resource = matched.resource.ifBlank { grant.resource },
            helperReason = matched.helperReason.ifBlank { grant.helperReason },
            suggestedStarter = matched.suggestedStarter ?: grant.suggestedStarter,
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

    @StringRes
    fun starterButtonRes(name: String): Int = when (name) {
        "mobile-dev" -> R.string.grants_allow_starter_mobile_dev
        "node-build" -> R.string.grants_allow_starter_node_build
        "rust-build" -> R.string.grants_allow_starter_rust_build
        else -> R.string.grants_allow_starter_generic
    }

    @StringRes
    fun starterDescriptionRes(name: String): Int? = when (name) {
        "mobile-dev" -> R.string.grants_starter_mobile_dev
        "node-build" -> R.string.grants_starter_node_build
        "rust-build" -> R.string.grants_starter_rust_build
        else -> null
    }

    fun profileAllowLines(profile: SandboxGrantProfile, format: (Int, String) -> String): List<String> = buildList {
        if (profile.writable.isNotEmpty()) add(format(R.string.grants_profile_writable, profile.writable.joinToString(", ")))
        if (profile.readable.isNotEmpty()) add(format(R.string.grants_profile_readable, profile.readable.joinToString(", ")))
        if (profile.loopbackPorts.isNotEmpty()) add(format(R.string.grants_profile_ports, profile.loopbackPorts.joinToString(", ")))
        if (profile.egress.isNotEmpty()) add(format(R.string.grants_profile_egress, profile.egress.joinToString(", ")))
    }

    /** Returns the string resource and two format arguments (scope target, resource) for a standing rule. */
    fun standingRuleFormat(rule: SandboxGrant): Triple<Int, String, String> {
        val isRepo = rule.scope == "repo" || rule.sessionId.startsWith(REPO_SCOPE_PREFIX)
        if (isRepo) {
            val repo = rule.repo.ifBlank { rule.sessionId.removePrefix(REPO_SCOPE_PREFIX) }
            val res = when (rule.capability) {
                "fs_write" -> R.string.grants_rule_repo_fs_write
                "fs_read" -> R.string.grants_rule_repo_fs_read
                "loopback" -> R.string.grants_rule_repo_loopback
                "egress" -> R.string.grants_rule_repo_egress
                else -> R.string.grants_rule_repo_other
            }
            return Triple(res, repo, rule.resource)
        }
        val task = rule.taskId?.takeIf { it.isNotBlank() } ?: rule.sessionId.removePrefix(TASK_SCOPE_PREFIX)
        val res = when (rule.capability) {
            "fs_write" -> R.string.grants_rule_task_fs_write
            "fs_read" -> R.string.grants_rule_task_fs_read
            "loopback" -> R.string.grants_rule_task_loopback
            "egress" -> R.string.grants_rule_task_egress
            else -> R.string.grants_rule_task_other
        }
        return Triple(res, task, rule.resource)
    }

    /** Formats the weekly summary line using the singular string when HQ allowed exactly 1 request. */
    fun formatSummary(response: SandboxSummaryResponse, format: (Int, Array<out Any>) -> String): String {
        val s = response.summary
        return if (s.approvedByPolicy == 1) {
            format(R.string.grants_settings_summary_one, arrayOf(response.days, s.askedOwner, s.refused))
        } else {
            format(R.string.grants_settings_summary, arrayOf(response.days, s.approvedByPolicy, s.askedOwner, s.refused))
        }
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
