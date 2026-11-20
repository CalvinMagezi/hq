package example.hq.spike.data

import java.net.URLEncoder
import org.json.JSONArray
import org.json.JSONObject

const val SANDBOX_GRANTS_SOURCE = "sandbox-grants"
const val NO_SANDBOX_PROFILE = "none"
const val TASK_SCOPE_PREFIX = "task:"
const val REPO_SCOPE_PREFIX = "repo:"

data class SandboxGrant(
    val id: String,
    val sessionId: String,
    val taskId: String?,
    val host: String,
    val repo: String,
    val capability: String,
    val resource: String,
    val state: String,
    val decidedBy: String,
    val helperReason: String,
    val decisionNote: String,
    val requestedAt: Long,
    val decidedAt: Long?,
    val appliedAt: Long?,
    val expiresAt: Long?,
    val revokedAt: Long?,
    val scope: String = "session",
    val suggestedStarter: String? = null,
) {
    val isResource: Boolean get() = capability != "profile"
    val isActive: Boolean get() = state == "active"
    val isPending: Boolean get() = state == "pending"
    val isStandingRule: Boolean
        get() = scope == "task" || scope == "repo" ||
            sessionId.startsWith(TASK_SCOPE_PREFIX) || sessionId.startsWith(REPO_SCOPE_PREFIX)
}

data class SandboxGrantProfile(
    val name: String,
    val writable: List<String>,
    val readable: List<String>,
    val loopbackPorts: List<Int>,
    val egress: List<String>,
)

data class SandboxGrantsSummary(
    val approvedByPolicy: Int,
    val approvedByOwner: Int,
    val askedOwner: Int,
    val declined: Int,
    val refused: Int,
    val standingRules: Int,
)

data class SandboxSummaryResponse(
    val days: Int,
    val summary: SandboxGrantsSummary,
)

/** Lists sandbox grants, optionally filtered to one helper session. */
suspend fun HqClient.sandboxGrants(session: String? = null): List<SandboxGrant> {
    val query = session?.let { "?session=${URLEncoder.encode(it, "UTF-8")}" }.orEmpty()
    return send(authed("/api/sandbox-grants$query").build()) { text ->
        JSONObject(text).optJSONArray("grants").objects().map(::parseSandboxGrant)
    }
}

/** Lists standing access rules (task and repository scope). */
suspend fun HqClient.sandboxGrantRules(): List<SandboxGrant> =
    send(authed("/api/sandbox-grants/rules").build()) { text ->
        JSONObject(text).optJSONArray("rules").objects().map(::parseSandboxGrant)
    }

/** Lists shipped starter sets so the UI can show what a starter includes beneath its button. */
suspend fun HqClient.sandboxGrantStarters(): List<SandboxGrantProfile> =
    send(authed("/api/sandbox-grants/starters").build()) { text ->
        JSONObject(text).optJSONArray("starters").objects().map(::parseSandboxGrantProfile)
    }

/** Counts sandbox access decisions over the last [days] days. */
suspend fun HqClient.sandboxGrantSummary(days: Int = 7): SandboxSummaryResponse =
    send(authed("/api/sandbox-grants/summary?days=$days").build()) { text ->
        val o = JSONObject(text)
        val s = o.optJSONObject("summary") ?: JSONObject()
        SandboxSummaryResponse(
            days = o.optInt("days", days),
            summary = SandboxGrantsSummary(
                approvedByPolicy = s.optInt("approved_by_policy", 0),
                approvedByOwner = s.optInt("approved_by_owner", 0),
                askedOwner = s.optInt("asked_owner", 0),
                declined = s.optInt("declined", 0),
                refused = s.optInt("refused", 0),
                standingRules = s.optInt("standing_rules", 0),
            ),
        )
    }

/** Revokes one active sandbox grant or standing rule. 404 means it is no longer live. */
suspend fun HqClient.revokeSandboxGrant(id: String): String =
    send(authed("/api/sandbox-grants/${URLEncoder.encode(id, "UTF-8")}/revoke").post(HqClient.EMPTY_JSON).build()) { text ->
        JSONObject(text).optString("revoked", id)
    }

/** Lists configured access profiles from agent_host.grants.profiles. */
suspend fun HqClient.sandboxGrantProfiles(): List<SandboxGrantProfile> =
    send(authed("/api/sandbox-grants/profiles").build()) { text ->
        JSONObject(text).optJSONArray("profiles").objects().map(::parseSandboxGrantProfile)
    }

/** Binds a session or task to a configured access profile, or 'none' to clear it. */
suspend fun HqClient.bindSandboxProfile(scope: String, id: String, name: String): String =
    send(
        authed("/api/sandbox-grants/profile")
            .post(json(JSONObject().put("scope", scope).put("id", id).put("name", name)))
            .build(),
    ) { text ->
        JSONObject(text).optString("bound", name)
    }

private fun parseSandboxGrant(o: JSONObject): SandboxGrant = SandboxGrant(
    id = o.getString("id"),
    sessionId = o.optString("session_id", ""),
    taskId = o.optStringOrNull("task_id"),
    host = o.optString("host", ""),
    repo = o.optString("repo", ""),
    capability = o.optString("capability", ""),
    resource = o.optString("resource", ""),
    state = o.optString("state", ""),
    decidedBy = o.optString("decided_by", ""),
    helperReason = o.optString("helper_reason", ""),
    decisionNote = o.optString("decision_note", ""),
    requestedAt = o.optLong("requested_at", 0L),
    decidedAt = o.optLongOrNull("decided_at"),
    appliedAt = o.optLongOrNull("applied_at"),
    expiresAt = o.optLongOrNull("expires_at"),
    revokedAt = o.optLongOrNull("revoked_at"),
    scope = o.optString("scope", "session").ifBlank { "session" },
    suggestedStarter = o.optStringOrNull("suggested_starter")?.ifBlank { null },
)

private fun parseSandboxGrantProfile(o: JSONObject): SandboxGrantProfile = SandboxGrantProfile(
    name = o.getString("name"),
    writable = o.optJSONArray("writable").strings(),
    readable = o.optJSONArray("readable").strings(),
    loopbackPorts = o.optJSONArray("loopback_ports").ints(),
    egress = o.optJSONArray("egress").strings(),
)

private fun JSONObject.optLongOrNull(key: String): Long? = if (!has(key) || isNull(key)) null else optLong(key)

private fun JSONArray?.ints(): List<Int> = if (this == null) emptyList() else (0 until length()).map { optInt(it) }
