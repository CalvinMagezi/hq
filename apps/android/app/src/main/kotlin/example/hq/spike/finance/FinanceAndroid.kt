package example.hq.spike.finance

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.util.Log
import example.hq.spike.data.HqClient
import example.hq.spike.data.HqUnavailable
import kotlinx.coroutines.runBlocking
import org.json.JSONObject

/** Reads only allowlisted SMS inbox rows via ContentResolver; callers run on a background thread. */
class AndroidSmsReader(context: Context) : SmsReader {
    private val app = context.applicationContext

    override fun hasPermission(): Boolean = hasPermission(app)

    override fun read(senders: List<String>, sinceMs: Long, maxMessages: Int): List<FinanceMessage> {
        val selection = FinanceSelection.build(senders, sinceMs)
        val out = ArrayList<FinanceMessage>()
        try {
            app.contentResolver.query(
                SMS_INBOX_URI,
                PROJECTION,
                selection.where,
                selection.args.toTypedArray(),
                SORT_ORDER,
            )?.use { c ->
                while (c.moveToNext() && out.size < maxMessages) {
                    out += FinanceMessage(c.getString(COL_ADDRESS).orEmpty(), c.getLong(COL_DATE), c.getString(COL_BODY).orEmpty())
                }
            }
        } catch (e: RuntimeException) {
            // Never propagate platform exception messages because they can quote query data.
            throw FinanceReadException(FinanceLimits.REASON_SMS_READ_FAILED)
        }
        return out
    }

    companion object {
        private val SMS_INBOX_URI: Uri = Uri.parse("content://sms/inbox")
        private val PROJECTION = arrayOf("address", "date", "body")
        private const val SORT_ORDER = "date DESC"
        private const val COL_ADDRESS = 0
        private const val COL_DATE = 1
        private const val COL_BODY = 2

        fun hasPermission(c: Context): Boolean = c.checkSelfPermission(Manifest.permission.READ_SMS) == PackageManager.PERMISSION_GRANTED
    }
}

/** Posts SMS batches to HQ's signed-in finance import route on the caller's background thread. */
class HqFinancePoster(private val client: HqClient) : FinancePoster {
    override fun postImport(actionId: String, payload: JSONObject): FinanceCounts = try {
        val response = runBlocking { client.postFinanceImport(actionId, payload) }
        FinanceCounts.fromJson(response) ?: throw FinanceUploadException(FinanceLimits.REASON_UPLOAD_FAILED)
    } catch (e: HqUnavailable) {
        throw FinanceUploadException(FinanceLimits.REASON_UPLOAD_FAILED)
    }
}

/** App-private preferences for the Finance switch, last import mark, counts, and recent request_id ledger. */
class FinancePrefs(context: Context) : FinancePrefsStore {
    private val prefs = context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    override fun isEnabled(): Boolean = prefs.getBoolean(KEY_ENABLED, false)

    fun setEnabled(enabled: Boolean) {
        val edit = prefs.edit().putBoolean(KEY_ENABLED, enabled)
        if (!enabled) {
            edit.remove(KEY_LAST_MARK)
                .remove(KEY_LAST_AT)
                .remove(KEY_HAS_LAST)
                .remove(KEY_SEEN)
                .remove(KEY_IGNORED)
                .remove(KEY_TRANSACTIONS)
                .remove(KEY_ADDED)
                .remove(KEY_WAITING)
        }
        edit.apply()
    }

    override fun lastMarkMs(): Long? = if (prefs.contains(KEY_LAST_MARK)) prefs.getLong(KEY_LAST_MARK, 0L) else null

    fun lastSummary(): FinanceLastImport? {
        if (!prefs.getBoolean(KEY_HAS_LAST, false)) return null
        val counts = FinanceCounts(
            seen = prefs.getInt(KEY_SEEN, 0),
            ignored = prefs.getInt(KEY_IGNORED, 0),
            transactions = prefs.getInt(KEY_TRANSACTIONS, 0),
            added = prefs.getInt(KEY_ADDED, 0),
            waitingForReview = prefs.getInt(KEY_WAITING, 0),
        )
        return FinanceLastImport(prefs.getLong(KEY_LAST_AT, 0L), counts)
    }

    override fun recordImport(markMs: Long?, importedAtMs: Long, counts: FinanceCounts) {
        val edit = prefs.edit()
            .putBoolean(KEY_HAS_LAST, true)
            .putLong(KEY_LAST_AT, importedAtMs)
            .putInt(KEY_SEEN, counts.seen)
            .putInt(KEY_IGNORED, counts.ignored)
            .putInt(KEY_TRANSACTIONS, counts.transactions)
            .putInt(KEY_ADDED, counts.added)
            .putInt(KEY_WAITING, counts.waitingForReview)
        val current = lastMarkMs()
        if (markMs != null && (current == null || markMs > current)) edit.putLong(KEY_LAST_MARK, markMs)
        edit.commit()
    }

    override fun hasRequestId(requestId: String): Boolean = prefs.contains(LEDGER_PREFIX + requestId)

    override fun rememberRequestId(requestId: String) {
        trimLedger()
        // Commit synchronously so a process death right after upload does not re-read on retry.
        prefs.edit().putBoolean(LEDGER_PREFIX + requestId, true).putLong(STAMP_PREFIX + requestId, System.currentTimeMillis()).commit()
    }

    private fun trimLedger() {
        val stamps = prefs.all.filterKeys { it.startsWith(STAMP_PREFIX) }.mapValues { (it.value as? Long) ?: 0L }
        if (stamps.size < LEDGER_MAX) return
        val edit = prefs.edit()
        stamps.entries.sortedBy { it.value }.take(stamps.size - LEDGER_MAX + 1).forEach {
            edit.remove(it.key).remove(LEDGER_PREFIX + it.key.removePrefix(STAMP_PREFIX))
        }
        edit.apply()
    }

    companion object {
        private const val FILE = "finance_actions"
        private const val KEY_ENABLED = "enabled"
        private const val KEY_LAST_MARK = "last_mark_ms"
        private const val KEY_LAST_AT = "last_import_at_ms"
        private const val KEY_HAS_LAST = "has_last_import"
        private const val KEY_SEEN = "last_seen"
        private const val KEY_IGNORED = "last_ignored"
        private const val KEY_TRANSACTIONS = "last_transactions"
        private const val KEY_ADDED = "last_added"
        private const val KEY_WAITING = "last_waiting_for_review"
        private const val LEDGER_PREFIX = "ledger_"
        private const val STAMP_PREFIX = "stamp_"
        private const val LEDGER_MAX = 200
        private const val TAG = "FinanceImport"

        fun androidLog(line: String) { Log.i(TAG, line) }
    }
}
