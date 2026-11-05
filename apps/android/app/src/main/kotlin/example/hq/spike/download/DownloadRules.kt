package example.hq.spike.download

import java.util.Locale

/** What kind of connection the phone is on, as far as spending money is concerned. */
enum class NetKind { None, Free, Mobile, Roaming, PaidWifi, UnknownWifi }

/** What the owner is asked before a download starts, in the words of the data ask. */
enum class AskKind { Mobile, Roaming, PaidWifi, AnyNetwork, StartOver, WifiQuestion }

/** What to do at the tap. */
sealed interface Decision {
    data object Start : Decision
    data class Ask(val kind: AskKind) : Decision
}

/** The download rule that needs no Android: when to ask, how much space, how to round and say sizes. */
object DownloadRules {
    /** 1 MB is 1,000,000 bytes, as Android's own storage screen counts. */
    const val MB = 1_000_000L

    /** Over this the rule applies; at or under it a download just starts. */
    const val BIG_BYTES = 10 * MB
    const val SPARE_BYTES = 10 * MB
    const val PARTIAL_DAYS = 14L
    const val DAY_MS = 24L * 60 * 60 * 1000

    /** Server errors retry after these waits, then the row says Try again. */
    val RETRY_MS = longArrayOf(30_000, 120_000, 600_000)

    fun isBig(size: Long) = size > BIG_BYTES

    /** Roaming always asks; consent covers the rest; then the connection decides. Nothing is fetched before the answer. */
    fun decide(size: Long, net: NetKind, askAnyNetwork: Boolean, consented: Boolean): Decision = when {
        !isBig(size) -> Decision.Start
        net == NetKind.None -> Decision.Start
        net == NetKind.Roaming -> Decision.Ask(AskKind.Roaming)
        consented -> Decision.Start
        askAnyNetwork -> Decision.Ask(if (net == NetKind.Mobile) AskKind.Mobile else AskKind.AnyNetwork)
        net == NetKind.Mobile -> Decision.Ask(AskKind.Mobile)
        net == NetKind.PaidWifi -> Decision.Ask(AskKind.PaidWifi)
        net == NetKind.UnknownWifi -> Decision.Ask(AskKind.WifiQuestion)
        else -> Decision.Start
    }

    /** A connection the owner pays for by the MB: mobile data, roaming, or a Wi-Fi said to be paid. */
    fun isPaid(net: NetKind) = net == NetKind.Mobile || net == NetKind.Roaming || net == NetKind.PaidWifi

    /**
     * Android's flags plus what the owner said about this Wi-Fi: [wifiAnswer] is true for paid, false for free, null for unseen.
     * Android calling a Wi-Fi metered counts as paid without a question.
     */
    fun classify(online: Boolean, cellular: Boolean, wifiLike: Boolean, metered: Boolean, roaming: Boolean, wifiAnswer: Boolean?): NetKind = when {
        !online -> NetKind.None
        cellular -> if (roaming) NetKind.Roaming else NetKind.Mobile
        metered -> if (wifiLike) NetKind.PaidWifi else NetKind.Mobile
        wifiLike -> when (wifiAnswer) { true -> NetKind.PaidWifi; false -> NetKind.Free; null -> NetKind.UnknownWifi }
        else -> NetKind.Free
    }

    /** One formula for every download: the file, its unpacked copy, and 10 MB to spare. */
    fun spaceNeeded(download: Long, unpacked: Long) = download + unpacked + SPARE_BYTES

    /** How much more room is wanted, or null when there is enough. */
    fun spaceShort(free: Long, needed: Long): Long? = (needed - free).takeIf { it > 0 }

    /** An update unpacks into Android's install session at its own size. */
    fun updateSpaceNeeded(size: Long) = spaceNeeded(size, size)

    /** Size: nearest whole MB. */
    fun sizeMb(bytes: Long): Long = Math.round(bytes.toDouble() / MB)

    /** Done: whole MB, one decimal below 10 ("3.4"); rounded down so it never claims more than it has. */
    fun doneText(bytes: Long): String =
        if (bytes < BIG_BYTES) String.format(Locale.ROOT, "%.1f", Math.floor(bytes / 100_000.0) / 10.0) else (bytes / MB).toString()

    /** Left: size minus done, rounded up. */
    fun leftMb(size: Long, done: Long): Long = (size - done).coerceAtLeast(0).let { (it + MB - 1) / MB }

    /** Percent, rounded down and never 100 until the check passes. */
    fun percent(done: Long, size: Long): Int = if (size <= 0) 0 else minOf(99L, done * 100 / size).toInt()

    /** Whole MB that went over a paid link, at least 1 when any byte did. */
    fun usedMb(paidBytes: Long): Long = if (paidBytes <= 0) 0 else maxOf(1L, Math.round(paidBytes.toDouble() / MB))

    /** Partials older than this, counted from the last byte, are deleted. */
    fun expired(lastByteAtMs: Long, nowMs: Long): Boolean = nowMs - lastByteAtMs > PARTIAL_DAYS * DAY_MS

    /** The wait before retry number [attempt] (0 based), or null once the row should say Try again. */
    fun retryAfter(attempt: Int): Long? = RETRY_MS.getOrNull(attempt)

    /** The ask's buttons stack at font scale 1.5 and up, or when a label is over 18 characters. */
    fun stacks(waitLabel: String, downloadLabel: String, fontScale: Float) = fontScale >= 1.5f || maxOf(waitLabel.length, downloadLabel.length) > 18
}
