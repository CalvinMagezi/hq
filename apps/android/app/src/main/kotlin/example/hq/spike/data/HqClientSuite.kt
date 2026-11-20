package example.hq.spike.data

import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject

// The HQ Suite calls of [HqClient], one extension per route. They are here so HqClient.kt stays
// the transport (auth, timeouts, errors) and each domain lives in its own file.

/** Where the office file at [path] stands for the HQ Suite (made smaller, waiting for an answer, or untouched). */
suspend fun HqClient.suiteFile(path: String): SuiteFile =
    send(authed("/api/suite/file?path=${java.net.URLEncoder.encode(path, "UTF-8")}").build()) { SuiteWire.file(JSONObject(it)) }

/** The `index`th `sheet` block of the note at [path], calculated and formatted by hq (a window of at most 30 rows by 10 columns). */
suspend fun HqClient.suiteSheet(path: String, index: Int): SheetView =
    send(authed("/api/suite/sheet?path=${java.net.URLEncoder.encode(path, "UTF-8")}&index=$index").build()) { SheetWire.view(JSONObject(it)) }

/**
 * Sets one cell of the `index`th `sheet` block (zero-based [row] and [col] in the sheet) and returns the sheet recalculated.
 * [expected] is what the cell held when it was shown; hq refuses the edit if it holds something else now.
 */
suspend fun HqClient.suiteEditCell(path: String, index: Int, row: Int, col: Int, expected: String, text: String): SheetView =
    SheetWire.view(postJson("/api/suite/sheet/cell", JSONObject().put("path", path).put("index", index).put("row", row).put("col", col).put("expected", expected).put("text", text)))

/** The blocks of the note at [path], in order, for the block editor. */
suspend fun HqClient.suiteBlocks(path: String): List<BlockRow> =
    send(authed("/api/suite/blocks?path=${java.net.URLEncoder.encode(path, "UTF-8")}").build()) { BlocksWire.list(JSONObject(it)) }

/** Replaces one block with [markdown]. hq refuses it if the block no longer hashes to [hash]; the blocks come back as they are now. */
suspend fun HqClient.suiteEditBlock(path: String, index: Int, hash: String, markdown: String): List<BlockRow> =
    BlocksWire.list(postJson("/api/suite/blocks/edit", JSONObject().put("path", path).put("index", index).put("hash", hash).put("markdown", markdown)))

/** Adds [markdown] after the block at [afterIndex] (which must still hash to [afterHash]), or at the top when [afterIndex] is null. */
suspend fun HqClient.suiteInsertBlock(path: String, afterIndex: Int?, afterHash: String?, markdown: String): List<BlockRow> =
    BlocksWire.list(postJson("/api/suite/blocks/insert", JSONObject().put("path", path).put("markdown", markdown).also { if (afterIndex != null) it.put("index", afterIndex).put("hash", afterHash ?: "") }))

suspend fun HqClient.suiteDeleteBlock(path: String, index: Int, hash: String): List<BlockRow> =
    BlocksWire.list(postJson("/api/suite/blocks/delete", JSONObject().put("path", path).put("index", index).put("hash", hash)))

/** The sinks this phone keeps sealed originals in, and whether each answers now. */
suspend fun HqClient.suiteSinks(): List<SinkInfo> = send(authed("/api/suite/sinks").build()) { SinkWire.list(JSONObject(it)) }

/** Makes a sink kept by a paired machine at [addr] (a loopback forward, such as 127.0.0.1:5700) and returns its recovery code, shown once and kept nowhere. */
suspend fun HqClient.suiteAddPeerSink(id: String, addr: String): String =
    postJson("/api/suite/sinks/peer", JSONObject().put("id", id).put("addr", addr)).optString("code", "")

/** Fetches a spread of the sink's originals and checks each opens and matches. The code is sent for this call only. */
suspend fun HqClient.suiteCheckSink(id: String, code: String): SinkCheck =
    postJson("/api/suite/sinks/check", JSONObject().put("id", id).put("code", code)).let { SinkCheck(it.optString("text", ""), it.optInt("failed", 0)) }

/** The `index`th `plot` block of the note at [path] as the chart JSON the chat charts use, computed by hq from the sheet range. */
suspend fun HqClient.suitePlot(path: String, index: Int): String =
    send(authed("/api/suite/plot?path=${java.net.URLEncoder.encode(path, "UTF-8")}&index=$index").build()) { it }

/** The `index`th `tile` block of the note at [path]: its label and the number hq computed. */
suspend fun HqClient.suiteTile(path: String, index: Int): TileView =
    send(authed("/api/suite/tile?path=${java.net.URLEncoder.encode(path, "UTF-8")}&index=$index").build()) { tileView(JSONObject(it)) }

/** Writes the smaller record and checks it. The original does not move until [suiteConfirm]. */
suspend fun HqClient.suiteShrink(path: String): SuiteFile = suiteAction("/api/suite/shrink", JSONObject().put("path", path))

suspend fun HqClient.suiteConfirm(job: String): SuiteFile = suiteAction("/api/suite/confirm", JSONObject().put("job", job))

/** Seals the prepared original into the sink and removes it from the phone once it reads back. The code is sent for this call only. */
suspend fun HqClient.suiteOffload(job: String, sink: String, code: String): SuiteFile =
    suiteAction("/api/suite/offload", JSONObject().put("job", job).put("sink", sink).put("code", code))

suspend fun HqClient.suiteAbandon(job: String): SuiteFile = suiteAction("/api/suite/abandon", JSONObject().put("job", job))

/** Brings the original back from the trash, or from its sink when [code] (the sink's recovery code, sent for this call only) is given. */
suspend fun HqClient.suiteUndo(job: String, code: String? = null): SuiteFile =
    suiteAction("/api/suite/undo", JSONObject().put("job", job).apply { code?.let { put("code", it) } })

/** A picture from the suite's media store, by its vault path (`_media/cas/...`); hq serves nothing else here. */
suspend fun HqClient.suiteMedia(path: String): ByteArray = withContext(Dispatchers.IO) {
    val request = authed("/api/suite/media?path=${java.net.URLEncoder.encode(path, "UTF-8")}").build()
    try {
        http.newCall(request).execute().use { r ->
            if (r.code == HqClient.HTTP_UNAUTHORIZED) refused()
            if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for /api/suite/media", status = r.code)
            r.body.bytes()
        }
    } catch (e: IOException) {
        throw if (e is HqUnavailable) e else HqUnavailable("hq is not reachable", e)
    }
}

/** A suite action: a 422 carries a sentence for the person, which [SuiteRefused] hands on. */
private suspend fun HqClient.suiteAction(route: String, body: JSONObject): SuiteFile = withContext(Dispatchers.IO) {
    val request = authed(route).post(body.toString().toRequestBody("application/json".toMediaType())).build()
    try {
        suiteHttp.newCall(request).execute().use { r ->
            if (r.code == HqClient.HTTP_UNAUTHORIZED) refused()
            val text = r.body.string()
            if (r.code == HqClient.HTTP_UNPROCESSABLE) throw SuiteRefused(SuiteWire.refusal(text) ?: "hq could not do that")
            if (!r.isSuccessful) throw HqUnavailable("HTTP ${r.code} for $route", status = r.code)
            SuiteWire.file(JSONObject(text))
        }
    } catch (e: IOException) {
        throw if (e is HqUnavailable || e is SuiteRefused) e else HqUnavailable("hq is not reachable", e)
    } catch (e: org.json.JSONException) {
        throw HqUnavailable("hq sent something unreadable", e, status = HqUnavailable.UNREADABLE)
    }
}

/**
 * One edit through hq's edit route (`POST /api/suite/edit`), quoting the hash of the `sheet` block read just before so hq refuses
 * a change made from a screen that is out of date. [sheet] counts `sheet` blocks from zero. A refusal throws with hq's sentence.
 */
private suspend fun HqClient.suiteSheetEdit(path: String, sheet: Int, action: String, build: (JSONObject) -> Unit) {
    val hash = suiteBlocks(path).filter { it.kind == "sheet" }.getOrNull(sheet)?.hash ?: throw HqUnavailable("that note has no such sheet")
    val body = JSONObject().put("path", path).put("action", action).put("sheet", sheet).put("hash", hash)
    build(body)
    postJson("/api/suite/edit", body)
}

/** Adds a blank row to the `sheet`th sheet of [path], before table row [at] (1 is the first row under the header), or at the end when [at] is null. */
suspend fun HqClient.suiteAddRow(path: String, sheet: Int, at: Int?) =
    suiteSheetEdit(path, sheet, "add_rows") { b ->
        b.put("rows", org.json.JSONArray().put(org.json.JSONArray().put("")))
        if (at != null) b.put("at", at)
    }

/** Adds a column named [name] on the right of the `sheet`th sheet of [path]. */
suspend fun HqClient.suiteAddColumn(path: String, sheet: Int, name: String) =
    suiteSheetEdit(path, sheet, "add_column") { it.put("name", name) }
