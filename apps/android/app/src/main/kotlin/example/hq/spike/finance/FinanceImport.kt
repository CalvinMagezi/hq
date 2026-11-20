package example.hq.spike.finance

import example.hq.spike.calendar.CalendarRequests
import example.hq.spike.deviceactions.DeviceOutcome
import org.json.JSONArray
import org.json.JSONObject

/** Bounds and wire constants of the finance.import contract; HQ checks them too, the phone does not trust that. */
object FinanceLimits {
    const val ACTION_NAME = "finance.import"
    const val WIRE_VERSION = 1
    const val MAX_MESSAGES_MIN = 1
    const val MAX_MESSAGES_LIMIT = 2_000
    const val SINCE_DAYS_MIN = 1
    const val SINCE_DAYS_MAX = 365
    const val FIRST_IMPORT_DAYS = 90
    const val OVERLAP_DAYS = 1
    const val MS_PER_DAY = 86_400_000L
    const val LEDGER_PATH = "_data/finance/ledger.md"
    const val LEDGER_LINK = "hq://finance/ledger"
    const val REASON_UNKNOWN_FIELD = "unknown_field"
    const val REASON_INVALID_REQUEST_ID = "invalid_request_id"
    const val REASON_INVALID_SENDERS = "invalid_senders"
    const val REASON_INVALID_MAX_MESSAGES = "invalid_max_messages"
    const val REASON_INVALID_SINCE_DAYS = "invalid_since_days"
    const val REASON_SMS_READ_FAILED = "sms_read_failed"
    const val REASON_UPLOAD_FAILED = "upload_failed"
    val ALLOWED_SENDERS: List<String> = listOf("MTNMobMoney", "MTNMoMo", "AIRTELMONEY", "AirtelMoney")
    val ALLOWED_SENDERS_LOWER: Set<String> = ALLOWED_SENDERS.map { it.lowercase() }.toSet()
}

/** A failure while reading SMS; message is always a fixed reason word so no data can leak. */
class FinanceReadException(val reason: String = FinanceLimits.REASON_SMS_READ_FAILED) : Exception(reason)

/** A failure while posting messages to HQ; message is always a fixed reason word so no data can leak. */
class FinanceUploadException(val reason: String = FinanceLimits.REASON_UPLOAD_FAILED) : Exception(reason)

/** One SMS row read from the inbox, kept in memory only for the upload. */
data class FinanceMessage(val sender: String, val dateMs: Long, val body: String)

/** Counts HQ returned after writing the ledger. */
data class FinanceCounts(
    val seen: Int,
    val ignored: Int,
    val transactions: Int,
    val added: Int,
    val waitingForReview: Int,
) {
    companion object {
        fun fromJson(o: JSONObject): FinanceCounts? {
            if (o.optString("outcome") != "imported") return null
            val seen = nonNegInt(o, "seen") ?: return null
            val ignored = nonNegInt(o, "ignored") ?: return null
            val transactions = nonNegInt(o, "transactions") ?: return null
            val added = nonNegInt(o, "added") ?: return null
            val waiting = nonNegInt(o, "waiting_for_review") ?: return null
            return FinanceCounts(seen, ignored, transactions, added, waiting)
        }

        private fun nonNegInt(o: JSONObject, key: String): Int? {
            val raw = o.opt(key)
            val n = when (raw) {
                is Int -> raw
                is Long -> if (raw in 0L..Int.MAX_VALUE.toLong()) raw.toInt() else return null
                else -> return null
            }
            return n.takeIf { it >= 0 }
        }
    }
}

/** The last successful import summary shown on Settings > Finance. */
data class FinanceLastImport(val importedAtMs: Long, val counts: FinanceCounts)

data class FinanceImportRequest(
    val requestId: String,
    val senders: List<String>,
    val maxMessages: Int,
    val sinceDays: Int?,
)

sealed interface ParsedFinanceRequest {
    data class Ok(val request: FinanceImportRequest) : ParsedFinanceRequest
    data class Bad(val outcome: FinanceOutcome.InvalidRequest) : ParsedFinanceRequest
}

/** SQL selection clause and bind arguments for querying content://sms/inbox. */
data class SmsSelection(val where: String, val args: List<String>)

/** Preferences and request-id ledger behind a seam for JVM tests. */
interface FinancePrefsStore {
    fun isEnabled(): Boolean
    fun lastMarkMs(): Long?
    fun recordImport(markMs: Long?, importedAtMs: Long, counts: FinanceCounts)
    fun hasRequestId(requestId: String): Boolean
    fun rememberRequestId(requestId: String)
}

/** The SMS inbox reader behind a seam so gating, selection and failure paths run on the JVM. */
interface SmsReader {
    fun hasPermission(): Boolean
    fun read(senders: List<String>, sinceMs: Long, maxMessages: Int): List<FinanceMessage>
}

/** The upload call behind a seam so the importer does not depend on OkHttp. */
interface FinancePoster {
    fun postImport(actionId: String, payload: JSONObject): FinanceCounts
}

object FinanceRequests {
    private val KNOWN = setOf("request_id", "senders", "max_messages", "since_days")

    fun parse(payload: JSONObject): ParsedFinanceRequest {
        val unknown = payload.keys().asSequence().firstOrNull { it !in KNOWN }
        if (unknown != null) return bad(FinanceLimits.REASON_UNKNOWN_FIELD)
        val requestId = payload.opt("request_id") as? String ?: return bad(FinanceLimits.REASON_INVALID_REQUEST_ID)
        if (!CalendarRequests.isUuid(requestId)) return bad(FinanceLimits.REASON_INVALID_REQUEST_ID)
        val senders = parseSenders(payload.opt("senders")) ?: return bad(FinanceLimits.REASON_INVALID_SENDERS)
        val maxMessages = exactInt(payload.opt("max_messages")) ?: return bad(FinanceLimits.REASON_INVALID_MAX_MESSAGES)
        if (maxMessages !in FinanceLimits.MAX_MESSAGES_MIN..FinanceLimits.MAX_MESSAGES_LIMIT) {
            return bad(FinanceLimits.REASON_INVALID_MAX_MESSAGES)
        }
        val sinceDays = parseSinceDays(payload) ?: return bad(FinanceLimits.REASON_INVALID_SINCE_DAYS)
        return ParsedFinanceRequest.Ok(FinanceImportRequest(requestId, senders, maxMessages, sinceDays.value))
    }

    private class OptInt(val value: Int?)

    private fun parseSinceDays(payload: JSONObject): OptInt? {
        if (!payload.has("since_days") || payload.isNull("since_days")) return OptInt(null)
        val days = exactInt(payload.opt("since_days")) ?: return null
        return if (days in FinanceLimits.SINCE_DAYS_MIN..FinanceLimits.SINCE_DAYS_MAX) OptInt(days) else null
    }

    private fun parseSenders(raw: Any?): List<String>? {
        val arr = raw as? JSONArray ?: return null
        if (arr.length() == 0) return null
        val out = ArrayList<String>(arr.length())
        for (i in 0 until arr.length()) {
            val s = arr.opt(i) as? String ?: return null
            if (s.isBlank() || s.lowercase() !in FinanceLimits.ALLOWED_SENDERS_LOWER) return null
            out += s
        }
        return out
    }

    private fun exactInt(raw: Any?): Int? = when (raw) {
        is Int -> raw
        is Long -> if (raw in Int.MIN_VALUE.toLong()..Int.MAX_VALUE.toLong()) raw.toInt() else null
        else -> null
    }

    private fun bad(reason: String) = ParsedFinanceRequest.Bad(FinanceOutcome.InvalidRequest(reason))
}

/** Chooses the lower bound timestamp (exclusive) for the SMS query. */
object FinanceWindow {
    fun startMs(nowMs: Long, sinceDays: Int?, lastImportMarkMs: Long?): Long {
        if (sinceDays != null) return (nowMs - sinceDays.toLong() * FinanceLimits.MS_PER_DAY).coerceAtLeast(0L)
        if (lastImportMarkMs != null) {
            return (lastImportMarkMs - FinanceLimits.OVERLAP_DAYS.toLong() * FinanceLimits.MS_PER_DAY).coerceAtLeast(0L)
        }
        return (nowMs - FinanceLimits.FIRST_IMPORT_DAYS.toLong() * FinanceLimits.MS_PER_DAY).coerceAtLeast(0L)
    }
}

/** Builds a parameterised selection so non-allowlisted messages are never loaded into memory. */
object FinanceSelection {
    fun build(senders: List<String>, sinceMs: Long): SmsSelection {
        require(senders.isNotEmpty()) { "senders must not be empty" }
        val placeholders = senders.joinToString(",") { "?" }
        return SmsSelection("address COLLATE NOCASE IN ($placeholders) AND date > ?", senders + sinceMs.toString())
    }
}

/** Serialises the messages for POST /api/finance/import/{action_id}. */
object FinanceUploadBody {
    fun toJson(messages: List<FinanceMessage>): JSONObject {
        val list = JSONArray()
        messages.forEach { m ->
            list.put(JSONObject().put("sender", m.sender).put("date", m.dateMs).put("body", m.body))
        }
        return JSONObject().put("messages", list)
    }
}

/** Outcomes reported by the finance module; [AlreadySettled] tells the handler not to post a result. */
sealed interface FinanceOutcome : DeviceOutcome {
    override fun toJson(): JSONObject = JSONObject().put("v", FinanceLimits.WIRE_VERSION).put("outcome", code).also { fill(it) }
    fun fill(o: JSONObject) {}

    data object NotSetUp : FinanceOutcome {
        override val code get() = "not_set_up"
    }

    data object PermissionMissing : FinanceOutcome {
        override val code get() = "permission_missing"
    }

    data class InvalidRequest(val reason: String) : FinanceOutcome {
        override val code get() = "invalid_request"
        override fun fill(o: JSONObject) { o.put("reason", reason) }
    }

    data class ReadFailed(val reason: String) : FinanceOutcome {
        override val code get() = "read_failed"
        override fun fill(o: JSONObject) { o.put("reason", reason) }
    }

    data class Unavailable(val reason: String) : FinanceOutcome {
        override val code get() = "finance_unavailable"
        override fun fill(o: JSONObject) { o.put("reason", reason) }
    }

    data class AlreadySettled(val counts: FinanceCounts?) : FinanceOutcome {
        override val code get() = "imported"
        override val alreadySettled get() = true
    }
}

/** Runs one finance.import action: gates, reads allowlisted SMS, posts to HQ, and stores only mark and counts. */
class FinanceImporter(
    private val prefs: FinancePrefsStore,
    private val reader: SmsReader,
    private val poster: FinancePoster,
    private val nowMs: () -> Long = System::currentTimeMillis,
    private val log: (String) -> Unit = {},
) {
    fun import(actionId: String, request: FinanceImportRequest): FinanceOutcome {
        if (prefs.hasRequestId(request.requestId)) {
            log("finance_import $actionId phase=duplicate")
            return FinanceOutcome.AlreadySettled(null)
        }
        if (!prefs.isEnabled()) {
            log("finance_import $actionId phase=not_set_up")
            return FinanceOutcome.NotSetUp
        }
        if (!reader.hasPermission()) {
            log("finance_import $actionId phase=permission_missing")
            return FinanceOutcome.PermissionMissing
        }
        val sinceMs = FinanceWindow.startMs(nowMs(), request.sinceDays, prefs.lastMarkMs())
        val messages = readMessages(actionId, request, sinceMs) ?: return FinanceOutcome.ReadFailed(FinanceLimits.REASON_SMS_READ_FAILED)
        val counts = uploadMessages(actionId, messages) ?: return FinanceOutcome.ReadFailed(FinanceLimits.REASON_UPLOAD_FAILED)
        val maxDate = messages.maxOfOrNull { it.dateMs }
        prefs.rememberRequestId(request.requestId)
        prefs.recordImport(maxDate, nowMs(), counts)
        log("finance_import $actionId phase=imported seen=${counts.seen} ignored=${counts.ignored} transactions=${counts.transactions} added=${counts.added} waiting_for_review=${counts.waitingForReview}")
        return FinanceOutcome.AlreadySettled(counts)
    }

    private fun readMessages(actionId: String, request: FinanceImportRequest, sinceMs: Long): List<FinanceMessage>? = try {
        val allowed = request.senders.map { it.lowercase() }.toSet()
        // Drop anything outside the request's senders even if a provider ignores COLLATE NOCASE.
        reader.read(request.senders, sinceMs, request.maxMessages)
            .filter { it.sender.lowercase() in allowed }
            .take(request.maxMessages)
    } catch (e: FinanceReadException) {
        log("finance_import $actionId phase=read_failed")
        null
    }

    private fun uploadMessages(actionId: String, messages: List<FinanceMessage>): FinanceCounts? = try {
        poster.postImport(actionId, FinanceUploadBody.toJson(messages))
    } catch (e: FinanceUploadException) {
        log("finance_import $actionId phase=upload_failed")
        null
    }
}
