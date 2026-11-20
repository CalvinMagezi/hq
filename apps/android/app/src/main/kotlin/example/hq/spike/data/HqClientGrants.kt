package example.hq.spike.data

import java.net.URLEncoder
import org.json.JSONArray
import org.json.JSONObject

const val SANDBOX_GRANTS_SOURCE = "sandbox-grants"
const val NO_SANDBOX_PROFILE = "none"

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
) {
    val isResource: Boolean get() = capability != "profile"
    val isActive: Boolean get() = state == "active"
    val isPending: Boolean get() = state == "pending"
}

data class SandboxGrantProfile(
    val name: String,
    val writable: List<String>,
    val readable: List<String>,
    val loopbackPorts: List<Int>,
    val egress: List<String>,
)

/** Lists sandbox grants, optionally filtered to one helper session. */
suspend fun HqClient.sandboxGrants(session: String? = null): List<SandboxGrant> {
    val query = session?.let { "?session=${URLEncoder.encode(it, "UTF-8")}" }.orEmpty()
    return send(authed("/api/sandbox-grants$query").build()) { text ->
        JSONObject(text).optJSONArray("grants").objects().map(::parseSandboxGrant)
    }
}

/** Revokes one active sandbox grant. 404 means the grant is no longer live. */
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
