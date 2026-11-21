package example.hq.spike.finance

import example.hq.spike.data.ChatEvent
import example.hq.spike.deviceactions.DeviceActionHandler
import example.hq.spike.deviceactions.DeviceActionLink
import example.hq.spike.deviceactions.DeviceActionModule
import example.hq.spike.deviceactions.DeviceActionRegistry
import example.hq.spike.deviceactions.DeviceOutcome
import java.time.Instant
import kotlinx.coroutines.runBlocking
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class FinanceImportTest {
    private val requestId = "3f2b8a52-6f0e-4d3e-9a55-0d0c3c1f7a99"
    private val nowMs = 1_700_000_000_000L

    private class FakePrefs(
        var enabled: Boolean = true,
        var markMs: Long? = null,
    ) : FinancePrefsStore {
        val seenRequests = mutableSetOf<String>()
        var lastCounts: FinanceCounts? = null
        var lastRecordedMark: Long? = null
        var recordCalls = 0

        override fun isEnabled() = enabled
        override fun lastMarkMs() = markMs
        override fun recordImport(markMs: Long?, importedAtMs: Long, counts: FinanceCounts) {
            recordCalls++
            lastCounts = counts
            if (markMs != null && (this.markMs == null || markMs > this.markMs!!)) {
                this.markMs = markMs
            }
            lastRecordedMark = markMs
        }
        override fun hasRequestId(requestId: String) = requestId in seenRequests
        override fun rememberRequestId(requestId: String) { seenRequests += requestId }
    }

    private class FakeReader(
        var allowed: Boolean = true,
        var messages: List<FinanceMessage> = emptyList(),
        var fail: Boolean = false,
    ) : SmsReader {
        var readCalls = 0
        var lastSenders: List<String> = emptyList()
        var lastSinceMs: Long = -1L
        var lastMax: Int = -1

        override fun hasPermission() = allowed
        override fun read(senders: List<String>, sinceMs: Long, maxMessages: Int): List<FinanceMessage> {
            readCalls++
            lastSenders = senders
            lastSinceMs = sinceMs
            lastMax = maxMessages
            if (fail) throw FinanceReadException(FinanceLimits.REASON_SMS_READ_FAILED)
            return messages
        }
    }

    private class FakePoster(
        var fail: Boolean = false,
        var counts: FinanceCounts = FinanceCounts(seen = 2, ignored = 0, transactions = 2, added = 1, waitingForReview = 1),
    ) : FinancePoster {
        var postCalls = 0
        var lastActionId: String? = null
        var lastPayload: JSONObject? = null

        override fun postImport(actionId: String, payload: JSONObject): FinanceCounts {
            postCalls++
            lastActionId = actionId
            lastPayload = payload
            if (fail) throw FinanceUploadException(FinanceLimits.REASON_UPLOAD_FAILED)
            return counts
        }
    }

    private fun validPayload(
        senders: List<String> = FinanceLimits.ALLOWED_SENDERS,
        maxMessages: Int = 2000,
        sinceDays: Any? = null,
    ): JSONObject = JSONObject()
        .put("request_id", requestId)
        .put("senders", JSONArray(senders))
        .put("max_messages", maxMessages)
        .also { if (sinceDays != null) it.put("since_days", sinceDays) }

    private fun validRequest(
        id: String = requestId,
        senders: List<String> = FinanceLimits.ALLOWED_SENDERS,
        maxMessages: Int = 2000,
        sinceDays: Int? = null,
    ) = FinanceImportRequest(id, senders, maxMessages, sinceDays)

    @Test fun rejectsUnknownField() {
        val p = validPayload().put("extra", "nope")
        val res = FinanceRequests.parse(p) as ParsedFinanceRequest.Bad
        assertEquals("invalid_request", res.outcome.code)
        assertEquals(FinanceLimits.REASON_UNKNOWN_FIELD, res.outcome.reason)
    }

    @Test fun rejectsBadSinceDays() {
        for (bad in listOf<Any>(0, -1, 366, "14", true, 14.5)) {
            val res = FinanceRequests.parse(validPayload(sinceDays = bad))
            assertTrue("expected Bad for $bad", res is ParsedFinanceRequest.Bad)
            assertEquals(FinanceLimits.REASON_INVALID_SINCE_DAYS, (res as ParsedFinanceRequest.Bad).outcome.reason)
        }
        val ok = FinanceRequests.parse(validPayload(sinceDays = 14)) as ParsedFinanceRequest.Ok
        assertEquals(14, ok.request.sinceDays)
        val absent = FinanceRequests.parse(validPayload()) as ParsedFinanceRequest.Ok
        assertNull(absent.request.sinceDays)
    }

    @Test fun rejectsDisallowedOrEmptySenders() {
        val badLists = listOf(
            emptyList(),
            listOf("BankOTP"),
            listOf("MTNMobMoney", "UnknownSender"),
            listOf(" "),
        )
        for (senders in badLists) {
            val res = FinanceRequests.parse(validPayload(senders = senders))
            assertTrue("expected Bad for $senders", res is ParsedFinanceRequest.Bad)
            assertEquals(FinanceLimits.REASON_INVALID_SENDERS, (res as ParsedFinanceRequest.Bad).outcome.reason)
        }
        val mixedCase = FinanceRequests.parse(validPayload(senders = listOf("mtnmobmoney", "AirtelMoney")))
        assertTrue(mixedCase is ParsedFinanceRequest.Ok)
    }

    @Test fun rejectsMaxMessagesOverLimitOrNonPositive() {
        for (bad in listOf(0, -5, 2001, 10000)) {
            val res = FinanceRequests.parse(validPayload(maxMessages = bad))
            assertTrue("expected Bad for $bad", res is ParsedFinanceRequest.Bad)
            assertEquals(FinanceLimits.REASON_INVALID_MAX_MESSAGES, (res as ParsedFinanceRequest.Bad).outcome.reason)
        }
        val ok = FinanceRequests.parse(validPayload(maxMessages = 2000))
        assertTrue(ok is ParsedFinanceRequest.Ok)
    }

    @Test fun gatingOrderChecksFinanceSwitchBeforePermissionAndNeverReads() {
        val prefs = FakePrefs(enabled = false)
        val reader = FakeReader(allowed = false)
        val poster = FakePoster()
        val importer = FinanceImporter(prefs, reader, poster, { nowMs })

        val off = importer.import("act-1", validRequest())
        assertEquals(FinanceOutcome.NotSetUp, off)
        assertEquals("not_set_up", off.toJson().getString("outcome"))
        assertEquals(0, reader.readCalls)
        assertEquals(0, poster.postCalls)

        prefs.enabled = true
        val noPerm = importer.import("act-2", validRequest())
        assertEquals(FinanceOutcome.PermissionMissing, noPerm)
        assertEquals("permission_missing", noPerm.toJson().getString("outcome"))
        assertEquals(0, reader.readCalls)
        assertEquals(0, poster.postCalls)
    }

    @Test fun dateWindowSinceDaysBeatsMarkBeatsNinetyDaysWithOverlap() {
        val mark = nowMs - 10L * FinanceLimits.MS_PER_DAY
        assertEquals(
            nowMs - 14L * FinanceLimits.MS_PER_DAY,
            FinanceWindow.startMs(nowMs, sinceDays = 14, lastImportMarkMs = mark),
        )
        assertEquals(
            mark - FinanceLimits.OVERLAP_DAYS * FinanceLimits.MS_PER_DAY,
            FinanceWindow.startMs(nowMs, sinceDays = null, lastImportMarkMs = mark),
        )
        assertEquals(
            nowMs - FinanceLimits.FIRST_IMPORT_DAYS * FinanceLimits.MS_PER_DAY,
            FinanceWindow.startMs(nowMs, sinceDays = null, lastImportMarkMs = null),
        )
    }

    @Test fun selectionBuilderRestrictsToPayloadSendersOnly() {
        val sel = FinanceSelection.build(listOf("MTNMobMoney", "AirtelMoney"), 123456L)
        assertEquals("address COLLATE NOCASE IN (?,?) AND date > ?", sel.where)
        assertEquals(listOf("MTNMobMoney", "AirtelMoney", "123456"), sel.args)
        assertFalse(sel.args.any { it.equals("OtherSender", ignoreCase = true) })
    }

    @Test fun repeatedRequestIdDoesNotReadOrPostTwice() {
        val prefs = FakePrefs(enabled = true)
        val reader = FakeReader(allowed = true, messages = listOf(FinanceMessage("MTNMobMoney", nowMs - 1000L, "You received UGX 5000")))
        val poster = FakePoster()
        val importer = FinanceImporter(prefs, reader, poster, { nowMs })

        val first = importer.import("act-1", validRequest())
        assertTrue(first is FinanceOutcome.AlreadySettled)
        assertEquals(1, reader.readCalls)
        assertEquals(1, poster.postCalls)

        val second = importer.import("act-2", validRequest())
        assertTrue(second is FinanceOutcome.AlreadySettled)
        assertEquals(1, reader.readCalls)
        assertEquals(1, poster.postCalls)
    }

    @Test fun emptyMessagesStillPostsAndDoesNotFail() {
        val prefs = FakePrefs(enabled = true)
        val reader = FakeReader(allowed = true, messages = emptyList())
        val poster = FakePoster(counts = FinanceCounts(0, 0, 0, 0, 0))
        val importer = FinanceImporter(prefs, reader, poster, { nowMs })

        val out = importer.import("act-empty", validRequest())
        assertTrue(out is FinanceOutcome.AlreadySettled)
        assertEquals(1, poster.postCalls)
        assertEquals(0, poster.lastPayload!!.getJSONArray("messages").length())
        assertNull(prefs.markMs)
        assertEquals(1, prefs.recordCalls)
    }

    @Test fun readAndUploadFailuresReturnReadFailedWithFixedReason() {
        val prefs = FakePrefs(enabled = true)
        val failRead = FinanceImporter(prefs, FakeReader(allowed = true, fail = true), FakePoster(), { nowMs })
        val readOut = failRead.import("act-r", validRequest()) as FinanceOutcome.ReadFailed
        assertEquals("read_failed", readOut.code)
        assertEquals(FinanceLimits.REASON_SMS_READ_FAILED, readOut.reason)
        assertEquals(FinanceLimits.REASON_SMS_READ_FAILED, readOut.toJson().getString("reason"))

        val failUpload = FinanceImporter(prefs, FakeReader(allowed = true), FakePoster(fail = true), { nowMs })
        val uploadOut = failUpload.import("act-u", validRequest()) as FinanceOutcome.ReadFailed
        assertEquals("read_failed", uploadOut.code)
        assertEquals(FinanceLimits.REASON_UPLOAD_FAILED, uploadOut.reason)
        assertEquals(FinanceLimits.REASON_UPLOAD_FAILED, uploadOut.toJson().getString("reason"))
    }

    @Test fun messageBodyNeverReachesLogger() {
        val secretBody = "CONFIDENTIAL TXN 99887766 UGX 450000 from Jane Doe 0772000111"
        val logs = mutableListOf<String>()
        val prefs = FakePrefs(enabled = true)
        val reader = FakeReader(
            allowed = true,
            messages = listOf(FinanceMessage("MTNMobMoney", nowMs - 5000L, secretBody)),
        )
        val poster = FakePoster(counts = FinanceCounts(1, 0, 1, 1, 1))
        val importer = FinanceImporter(prefs, reader, poster, { nowMs }, { logs += it })

        importer.import("act-log-1", validRequest())

        assertTrue(logs.isNotEmpty())
        for (line in logs) {
            assertFalse("body leaked to log: $line", line.contains("CONFIDENTIAL"))
            assertFalse("txn id leaked to log: $line", line.contains("99887766"))
            assertFalse("amount leaked to log: $line", line.contains("450000"))
            assertFalse("name leaked to log: $line", line.contains("Jane Doe"))
            assertFalse("sender leaked to log: $line", line.contains("MTNMobMoney"))
        }
    }

    @Test fun handlerSkipsResultPostWhenServerAlreadySettledImport() = runBlocking {
        val acks = mutableListOf<String>()
        val results = mutableListOf<Pair<String, JSONObject>>()
        val link = object : DeviceActionLink {
            override suspend fun ack(actionId: String) { acks += actionId }
            override suspend fun result(actionId: String, result: JSONObject) { results += actionId to result }
            override suspend fun pending() = emptyList<ChatEvent.DeviceAction>()
        }
        val prefs = FakePrefs(enabled = true)
        val reader = FakeReader(allowed = true, messages = listOf(FinanceMessage("MTNMobMoney", nowMs - 1000L, "secret body")))
        val poster = FakePoster()
        val module = FinanceModule(FinanceImporter(prefs, reader, poster, { nowMs }))
        val handler = DeviceActionHandler(link, DeviceActionRegistry(listOf(module)), { Instant.ofEpochMilli(nowMs) }, {})

        val action = ChatEvent.DeviceAction("act-ok", "t1", FinanceLimits.ACTION_NAME, 1, validPayload(), null)
        handler.handle(action)

        assertEquals(listOf("act-ok"), acks)
        assertEquals(1, poster.postCalls)
        assertTrue("must not post /result after HTTP 200 upload", results.isEmpty())
    }

    @Test fun moduleClaimsOnlyFinanceImportAndRejectsDuplicateRegistration() {
        val module = FinanceModule(FinanceImporter(FakePrefs(), FakeReader(), FakePoster()))
        val duplicate = object : DeviceActionModule {
            override val actions = setOf(FinanceLimits.ACTION_NAME)
            override val wireVersion = 1
            override fun run(action: ChatEvent.DeviceAction): DeviceOutcome = error("unused")
        }
        try {
            DeviceActionRegistry(listOf(module, duplicate))
            fail("expected duplicate action registration to fail")
        } catch (expected: IllegalArgumentException) {
            assertTrue(expected.message!!.contains(FinanceLimits.ACTION_NAME))
        }
    }
}
