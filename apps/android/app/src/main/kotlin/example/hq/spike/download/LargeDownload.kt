package example.hq.spike.download

import example.hq.spike.measure.DataPurpose
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.security.MessageDigest
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONException
import org.json.JSONObject

/** One large download: a signed size and SHA-256 come with it, so the finished file can be checked before anything uses it. */
data class Item(
    val id: String,
    /** What the row and the notice call it, for example "AgentHQ 1.5.0". */
    val name: String,
    val url: String,
    val size: Long,
    val sha256: String,
    val purpose: DataPurpose,
    /** The room the file needs once unpacked; an update unpacks at its own size. */
    val unpacked: Long,
    val dest: String,
    val speech: Boolean = false,
)

/** Every state a download row or notice can be in. */
sealed interface DlState {
    data object Ready : DlState
    data class Asking(val kind: AskKind, val done: Long = 0) : DlState
    data class NoSpace(val need: Long, val short: Long) : DlState
    data object WaitingWifi : DlState
    data class Downloading(val done: Long, val onPaid: Boolean) : DlState
    data class PausedWifi(val done: Long) : DlState
    data class PausedOwner(val done: Long) : DlState
    data class CutOff(val done: Long) : DlState
    data object Checking : DlState
    data class Done(val paidBytes: Long) : DlState
    data object Mismatch : DlState
    data class ServerFailing(val done: Long) : DlState
    data class Cancelled(val done: Long) : DlState
}

/** What a server answered to a (maybe ranged) request. [rangeStart] is where a 206's body begins. */
class Opened(val status: Int, val rangeStart: Long, val etag: String?, val body: InputStream?, val close: () -> Unit = {})

/** Everything the engine needs from the phone, so the rules run the same on the JVM. */
interface DownloadEnv {
    fun now(): Long
    fun net(): NetKind
    fun askAnyNetwork(): Boolean
    fun freeBytes(): Long
    fun sleep(ms: Long)
    fun undoMs(): Long
    fun open(url: String, from: Long, ifRange: String?): Opened
    fun count(purpose: DataPurpose, paid: Boolean, bytes: Long)
    fun rememberWifi(paid: Boolean)
    fun later(delayMs: Long, run: () -> Unit)
    fun spawn(run: () -> Unit)
}

/**
 * The one engine for every download over 10 MB. It keeps the bytes in `dest.part`, resumes with a Range request, and trusts
 * the file only when it matches the signed size and SHA-256. Spending data is the owner's choice: nothing is fetched on a paid
 * connection before the answer, and leaving Wi-Fi pauses with the bytes kept.
 */
class LargeDownload(val item: Item, private val dir: File, private val env: DownloadEnv, private val onChange: (Item, DlState) -> Unit = { _, _ -> }) {
    private val part = File(dir, item.dest + ".part")
    private val meta = File(dir, item.dest + ".json")
    val file = File(dir, item.dest)

    private val flow = MutableStateFlow<DlState>(DlState.Ready)
    val state: StateFlow<DlState> = flow

    @Volatile private var stop = Stop.None
    @Volatile private var running = false
    private var consent = false
    private var paidBytes = 0L
    private var etag: String? = null
    private var cancelToken = 0
    private var lastEmit = 0L

    private enum class Stop { None, Pause, Cancel }

    init {
        load()
    }

    private fun set(s: DlState) {
        flow.value = s
        onChange(item, s)
    }

    fun kept(): Long = if (part.exists()) part.length() else 0L

    /** The owner tapped Download. The space check comes first, then the ask, and only then bytes. */
    @Synchronized fun request() {
        if (running || flow.value is DlState.Checking) return
        val have = kept()
        val short = DownloadRules.spaceShort(env.freeBytes(), DownloadRules.spaceNeeded(item.size - have, item.unpacked))
        if (short != null && DownloadRules.isBig(item.size)) {
            set(DlState.NoSpace(DownloadRules.spaceNeeded(item.size - have, item.unpacked), short))
            return
        }
        when (val d = DownloadRules.decide(item.size, env.net(), env.askAnyNetwork(), consent)) {
            Decision.Start -> begin()
            is Decision.Ask -> set(DlState.Asking(d.kind, have))
        }
    }

    /** The ask's answer: Download · {size} is consent for this download only. */
    @Synchronized fun accept() {
        val ask = flow.value as? DlState.Asking ?: return
        when (ask.kind) {
            AskKind.WifiQuestion -> return
            AskKind.StartOver -> { part.delete(); etag = null }
            else -> {}
        }
        consent = true
        begin()
    }

    /** Wait for Wi-Fi: saved, shown as waiting, started by itself on a free network. */
    @Synchronized fun waitForWifi() {
        if (flow.value !is DlState.Asking && flow.value !is DlState.PausedWifi && flow.value !is DlState.CutOff) return
        consent = false
        save("waiting")
        set(DlState.WaitingWifi)
        if (env.net() == NetKind.Free) begin()
    }

    /** Back or a tap outside the ask: nothing is fetched and nothing is saved. */
    @Synchronized fun dismissAsk() {
        if (flow.value is DlState.Asking) set(if (kept() > 0) DlState.PausedOwner(kept()) else DlState.Ready)
    }

    /** "Is this Wi-Fi paid by the MB?" It's paid, or it's free; Back is [dismissAsk] and asks again next time. */
    @Synchronized fun answerWifi(paid: Boolean) {
        if (flow.value != DlState.Asking(AskKind.WifiQuestion, kept())) return
        env.rememberWifi(paid)
        set(DlState.Ready)
        request()
    }

    /** "Download · {left}" on a paused or cut-off row is the consent; roaming asks again. */
    @Synchronized fun resumeWithConsent() {
        if (running) return
        if (env.net() == NetKind.Roaming) { set(DlState.Asking(AskKind.Roaming, kept())); return }
        consent = true
        begin()
    }

    /** Resume after the owner paused: the consent stands, unless roaming began. */
    @Synchronized fun resume() {
        if (flow.value !is DlState.PausedOwner || running) return
        when (DownloadRules.decide(item.size, env.net(), env.askAnyNetwork(), consent)) {
            Decision.Start -> begin()
            is Decision.Ask -> set(DlState.Asking(if (env.net() == NetKind.Roaming) AskKind.Roaming else AskKind.AnyNetwork, kept()))
        }
    }

    fun pause() {
        if (running) stop = Stop.Pause else synchronized(this) { if (flow.value is DlState.WaitingWifi) { save("paused"); set(DlState.PausedOwner(kept())) } }
    }

    /** Acts at once and never asks; Undo stays for the accessibility timeout, then the partial file goes. */
    fun cancel() {
        if (running) { stop = Stop.Cancel; return }
        synchronized(this) { cancelled(kept()) }
    }

    private fun cancelled(done: Long) {
        save("none")
        set(DlState.Cancelled(done))
        val token = ++cancelToken
        env.later(env.undoMs()) { synchronized(this) { if (cancelToken == token && flow.value is DlState.Cancelled) discard() } }
    }

    /** Undo after Cancel: goes on from the kept bytes. */
    @Synchronized fun undo() {
        if (flow.value !is DlState.Cancelled) return
        cancelToken++
        set(DlState.Ready)
        request()
    }

    /** Removes the partial file and everything saved about it. */
    @Synchronized fun discard() {
        part.delete(); meta.delete()
        consent = false; paidBytes = 0; etag = null
        set(DlState.Ready)
    }

    /** The phone's network changed: a waiting download starts on a free one, a cut-off one goes on when back online. */
    @Synchronized fun networkChanged() {
        val net = env.net()
        when (flow.value) {
            DlState.WaitingWifi -> if (net == NetKind.Free) begin()
            is DlState.CutOff -> when {
                net == NetKind.None -> {}
                net == NetKind.Roaming || (DownloadRules.isPaid(net) && !consent) -> set(DlState.PausedWifi(kept()))
                else -> begin()
            }
            else -> {}
        }
    }

    /** At hub start: a saved request comes back as it was; partial files past 14 days are removed. */
    @Synchronized fun restore() {
        if (part.exists() && DownloadRules.expired(lastByte(), env.now())) { discard(); return }
        when (wanted()) {
            "running" -> { set(DlState.CutOff(kept())); networkChanged() }
            "waiting" -> { set(DlState.WaitingWifi); networkChanged() }
            "paused" -> set(DlState.PausedOwner(kept()))
            else -> if (kept() > 0) set(DlState.PausedOwner(kept()))
        }
    }

    fun lastByte(): Long = if (part.exists()) part.lastModified() else 0L

    private fun begin() {
        running = true
        stop = Stop.None
        save("running")
        set(DlState.Downloading(kept(), DownloadRules.isPaid(env.net())))
        env.spawn { transfer() }
    }

    private sealed interface Outcome {
        data object Full : Outcome
        data object Stopped : Outcome
        data object Left : Outcome
        data object Failed : Outcome
        data object Mismatch : Outcome
    }

    private fun transfer() {
        try {
            var attempt = 0
            while (true) {
                if (!pathOk()) return
                when (attemptOnce()) {
                    Outcome.Full -> { verify(); return }
                    Outcome.Stopped -> { stopped(); return }
                    Outcome.Left -> return
                    Outcome.Mismatch -> { part.delete(); meta.delete(); set(DlState.Mismatch); return }
                    Outcome.Failed -> {
                        if (env.net() == NetKind.None) { set(DlState.CutOff(kept())); return }
                        val wait = DownloadRules.retryAfter(attempt++)
                        if (wait == null) { save("none"); set(DlState.ServerFailing(kept())); return }
                        try { env.sleep(wait) } catch (_: InterruptedException) { return }
                        if (stop != Stop.None) { stopped(); return }
                    }
                }
            }
        } finally {
            running = false
        }
    }

    /** False when the connection is gone or may not be used: the bytes stay and the row says why. */
    private fun pathOk(): Boolean {
        val net = env.net()
        return when {
            net == NetKind.None -> { set(DlState.CutOff(kept())); false }
            net == NetKind.Roaming || (DownloadRules.isPaid(net) && !consent && DownloadRules.isBig(item.size)) -> { set(DlState.PausedWifi(kept())); false }
            else -> true
        }
    }

    private fun stopped() {
        if (stop == Stop.Cancel) cancelled(kept()) else { save("paused"); set(DlState.PausedOwner(kept())) }
    }

    private fun attemptOnce(): Outcome {
        var from = kept()
        if (from > item.size) { part.delete(); from = 0 }
        if (from == item.size) return Outcome.Full
        val opened = try { env.open(item.url, from, if (from > 0) etag else null) } catch (_: IOException) { return Outcome.Failed }
        var segment = 0L
        var paidSegment = false
        try {
            var append = true
            when {
                opened.status == 206 && opened.rangeStart == from -> {}
                opened.status == 200 && from == 0L -> append = false
                opened.status == 200 || opened.status == 206 || opened.status == 416 -> {
                    // The server cannot go on from here: quietly start over on a free network, ask again on a paid one.
                    if (opened.status == 416 || DownloadRules.isPaid(env.net())) {
                        etag = null
                        consent = false
                        set(DlState.Asking(AskKind.StartOver, from))
                        return Outcome.Left
                    }
                    part.delete(); from = 0; append = false
                }
                else -> return Outcome.Failed
            }
            etag = opened.etag ?: etag
            val body = opened.body ?: return Outcome.Failed
            var netNow = env.net()
            var checked = env.now()
            java.io.FileOutputStream(part, append).use { out ->
                val buffer = ByteArray(BUFFER_BYTES)
                while (true) {
                    when (stop) {
                        Stop.None -> {}
                        else -> return Outcome.Stopped
                    }
                    val n = try { body.read(buffer) } catch (_: IOException) { return Outcome.Failed }
                    if (n < 0) break
                    if (part.length() + n > item.size) return Outcome.Mismatch
                    out.write(buffer, 0, n)
                    segment += n
                    paidSegment = DownloadRules.isPaid(netNow)
                    if (paidSegment) paidBytes += n
                    val t = env.now()
                    if (t - checked >= NET_CHECK_MS) {
                        checked = t
                        netNow = env.net()
                        if (netNow == NetKind.None || netNow == NetKind.Roaming || (DownloadRules.isPaid(netNow) && !consent)) {
                            out.flush()
                            set(if (netNow == NetKind.None) DlState.CutOff(kept()) else DlState.PausedWifi(kept()))
                            return Outcome.Left
                        }
                    }
                    if (t - lastEmit >= EMIT_MS) { lastEmit = t; set(DlState.Downloading(from + segment, DownloadRules.isPaid(netNow))) }
                }
            }
            return if (kept() == item.size) Outcome.Full else Outcome.Failed
        } finally {
            if (segment > 0) env.count(item.purpose, paidSegment, segment)
            opened.close()
            save(wanted())
        }
    }

    private fun verify() {
        set(DlState.Checking)
        val digest = MessageDigest.getInstance("SHA-256")
        part.inputStream().use { input ->
            val buffer = ByteArray(BUFFER_BYTES)
            while (true) { val n = input.read(buffer); if (n < 0) break; digest.update(buffer, 0, n) }
        }
        val hex = digest.digest().joinToString("") { "%02x".format(it) }
        if (part.length() != item.size || hex != item.sha256.lowercase()) {
            part.delete(); meta.delete()
            set(DlState.Mismatch)
            return
        }
        file.delete()
        if (!part.renameTo(file)) { part.delete(); set(DlState.ServerFailing(0)); return }
        meta.delete()
        set(DlState.Done(paidBytes))
    }

    private fun wanted(): String = try { JSONObject(meta.readText()).optString("wanted", "none") } catch (_: IOException) { "none" } catch (_: JSONException) { "none" }

    private fun save(wanted: String) {
        if (wanted == "none" && !part.exists()) { meta.delete(); return }
        dir.mkdirs()
        val o = JSONObject().put("wanted", wanted).put("consent", consent).put("paid", paidBytes).put("etag", etag ?: JSONObject.NULL)
            .put("id", item.id).put("name", item.name).put("url", item.url).put("size", item.size).put("sha256", item.sha256)
            .put("purpose", item.purpose.name).put("unpacked", item.unpacked).put("dest", item.dest).put("speech", item.speech)
        meta.writeText(o.toString())
    }

    private fun load() {
        try {
            val o = JSONObject(meta.readText())
            consent = o.optBoolean("consent", false)
            paidBytes = o.optLong("paid", 0)
            etag = if (o.isNull("etag")) null else o.optString("etag")
        } catch (_: IOException) {
        } catch (_: JSONException) {
        }
    }

    companion object {
        private const val BUFFER_BYTES = 64 * 1024
        private const val EMIT_MS = 250L
        private const val NET_CHECK_MS = 1000L

        /** The item a saved request describes, or null when its file is unreadable. */
        fun readItem(meta: File): Item? = try {
            val o = JSONObject(meta.readText())
            Item(o.getString("id"), o.getString("name"), o.getString("url"), o.getLong("size"), o.getString("sha256"), DataPurpose.valueOf(o.getString("purpose")), o.getLong("unpacked"), o.getString("dest"), o.optBoolean("speech", false))
        } catch (_: IOException) {
            null
        } catch (_: JSONException) {
            null
        } catch (_: IllegalArgumentException) {
            null
        }
    }
}
