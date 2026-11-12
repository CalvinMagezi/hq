package example.hq.spike.contacts

import example.hq.spike.calendar.CalendarRequests
import example.hq.spike.deviceactions.DeviceOutcome
import org.json.JSONArray
import org.json.JSONObject

/** Bounds of the contacts.search contract; HQ checks them too, the phone does not trust that. */
object ContactsLimits {
    const val ACTION_NAME = "contacts.search"
    const val WIRE_VERSION = 1
    const val QUERY_MIN = 2
    const val QUERY_MAX = 64
    const val LIMIT_MIN = 1
    const val LIMIT_MAX = 10
    const val LIMIT_DEFAULT = 5
    const val MAX_PHONES = 3
    const val MAX_EMAILS = 3
    const val NAME_MAX = 100
    const val LABEL_MAX = 30
    const val NUMBER_MAX = 32
    const val ADDRESS_MAX = 254
    const val MISSING_READ = "read_contacts"
    const val NEXT_STEP_PERMISSION = "Ask the owner to allow contacts access in HQ's phone settings, under Contacts."
    const val REASON_PROVIDER_ERROR = "provider_error"
}

/** A failure of the Contacts Provider that the caller turns into contacts_unavailable. */
class ContactsStoreException(message: String, cause: Throwable? = null) : Exception(message, cause)

/** What the provider gives back for one match, before it is cleaned and capped. */
data class ContactHit(val id: Long, val displayName: String)
data class RawPhone(val label: String, val number: String)
data class RawEmail(val label: String, val address: String)

/** The Contacts Provider behind a seam, so escaping, caps and outcomes run on the JVM against a fake. */
interface ContactsReader {
    fun hasPermission(): Boolean

    /** At most [limit] contacts whose display name matches the SQL LIKE [pattern] (backslash escapes), ordered by display name. */
    fun findByName(pattern: String, limit: Int): List<ContactHit>

    fun phones(contactIds: List<Long>): Map<Long, List<RawPhone>>
    fun emails(contactIds: List<Long>): Map<Long, List<RawEmail>>
}

data class ContactsSearchRequest(val requestId: String, val query: String, val limit: Int)

sealed interface ParsedContactsRequest {
    data class Ok(val request: ContactsSearchRequest) : ParsedContactsRequest
    data class Bad(val outcome: ContactsOutcome.InvalidQuery) : ParsedContactsRequest
}

/** Text the phone sends back came from other apps and other people, so it carries no control characters and has a length ceiling. */
object ContactText {
    fun clean(raw: String, max: Int): String = raw.filterNot { Character.isISOControl(it) || it == ' ' || it == ' ' }.trim().take(max)
}

object ContactsRequests {
    private val KNOWN = setOf("request_id", "query", "limit")

    fun parse(payload: JSONObject): ParsedContactsRequest {
        val unknown = payload.keys().asSequence().firstOrNull { it !in KNOWN }
        if (unknown != null) return bad("unknown_field", unknown)
        val requestId = payload.optString("request_id")
        if (!CalendarRequests.isUuid(requestId)) return bad("invalid_request_id", "request_id")
        val query = ContactText.clean(payload.optString("query"), Int.MAX_VALUE)
        if (query.length < ContactsLimits.QUERY_MIN || query.length > ContactsLimits.QUERY_MAX) return bad("query_length", "query")
        val limit = payload.optInt("limit", ContactsLimits.LIMIT_DEFAULT)
        if (limit < ContactsLimits.LIMIT_MIN) return bad("invalid_limit", "limit")
        return ParsedContactsRequest.Ok(ContactsSearchRequest(requestId, query, minOf(limit, ContactsLimits.LIMIT_MAX)))
    }

    /** A substring LIKE pattern that matches the query literally: `%`, `_` and the escape character itself lose their meaning. */
    fun likePattern(query: String): String {
        val escaped = buildString {
            query.forEach { if (it == '\\' || it == '%' || it == '_') append('\\'); append(it) }
        }
        return "%$escaped%"
    }

    private fun bad(reason: String, field: String) = ParsedContactsRequest.Bad(ContactsOutcome.InvalidQuery(reason, field))
}

data class ContactPhone(val label: String, val number: String)
data class ContactEmail(val label: String, val address: String)
data class ContactRef(val id: Long, val displayName: String, val phones: List<ContactPhone>, val emails: List<ContactEmail>)

/** What the phone reports back; the shapes are the contract's, so HQ can pass them on unchanged. */
sealed interface ContactsOutcome : DeviceOutcome {
    override fun toJson(): JSONObject = JSONObject().put("v", ContactsLimits.WIRE_VERSION).put("outcome", code).also { fill(it) }
    fun fill(o: JSONObject)

    data class Listed(val contacts: List<ContactRef>, val truncated: Boolean) : ContactsOutcome {
        override val code get() = "listed"
        override fun fill(o: JSONObject) {
            val rows = JSONArray()
            contacts.forEach { c ->
                val phones = JSONArray().also { a -> c.phones.forEach { a.put(JSONObject().put("label", it.label).put("number", it.number)) } }
                val emails = JSONArray().also { a -> c.emails.forEach { a.put(JSONObject().put("label", it.label).put("address", it.address)) } }
                rows.put(JSONObject().put("id", c.id).put("display_name", c.displayName).put("phones", phones).put("emails", emails))
            }
            o.put("contacts", rows).put("truncated", truncated)
        }
    }

    data class PermissionDenied(val missing: List<String>, val nextStep: String) : ContactsOutcome {
        override val code get() = "permission_denied"
        override fun fill(o: JSONObject) { o.put("missing", JSONArray(missing)).put("next_step", nextStep) }
    }

    data class InvalidQuery(val reason: String, val field: String? = null) : ContactsOutcome {
        override val code get() = "invalid_query"
        override fun fill(o: JSONObject) { o.put("reason", reason); field?.let { o.put("field", it) } }
    }

    data class Unavailable(val reason: String) : ContactsOutcome {
        override val code get() = "contacts_unavailable"
        override fun fill(o: JSONObject) { o.put("reason", reason) }
    }
}

/** The app-side steps of one contacts.search. Pure apart from the reader. */
class ContactsSearcher(private val reader: ContactsReader) {

    fun search(request: ContactsSearchRequest): ContactsOutcome {
        if (!reader.hasPermission()) return ContactsOutcome.PermissionDenied(listOf(ContactsLimits.MISSING_READ), ContactsLimits.NEXT_STEP_PERMISSION)
        return try {
            // One more than asked tells whether the answer is cut, without a second count query.
            val hits = reader.findByName(ContactsRequests.likePattern(request.query), request.limit + 1)
            val page = hits.take(request.limit)
            val ids = page.map { it.id }
            val phones = if (ids.isEmpty()) emptyMap() else reader.phones(ids)
            val emails = if (ids.isEmpty()) emptyMap() else reader.emails(ids)
            ContactsOutcome.Listed(page.map { ref(it, phones[it.id].orEmpty(), emails[it.id].orEmpty()) }, hits.size > request.limit)
        } catch (e: ContactsStoreException) {
            ContactsOutcome.Unavailable(ContactsLimits.REASON_PROVIDER_ERROR)
        }
    }

    private fun ref(hit: ContactHit, phones: List<RawPhone>, emails: List<RawEmail>) = ContactRef(
        hit.id,
        ContactText.clean(hit.displayName, ContactsLimits.NAME_MAX),
        phones.map { ContactPhone(ContactText.clean(it.label, ContactsLimits.LABEL_MAX), ContactText.clean(it.number, ContactsLimits.NUMBER_MAX)) }
            .filter { it.number.isNotEmpty() }.distinct().take(ContactsLimits.MAX_PHONES),
        emails.map { ContactEmail(ContactText.clean(it.label, ContactsLimits.LABEL_MAX), ContactText.clean(it.address, ContactsLimits.ADDRESS_MAX)) }
            .filter { it.address.isNotEmpty() }.distinct().take(ContactsLimits.MAX_EMAILS),
    )
}
