package example.hq.spike.update

import android.content.pm.PackageInstaller
import org.json.JSONException
import org.json.JSONObject

/** What the signed channel manifest (`android-channel.json`) says the newest APK is. */
data class ChannelManifest(
    val version: String,
    val versionCode: Long,
    val url: String,
    val sha256: String,
    val minSdk: Int,
    /** Rises with every manifest; one lower than the highest seen is a replay and is refused. */
    val seq: Long = 0,
    val issuedAt: String = "",
    /** The percent of phones that are offered this build by themselves, 0 to 100. */
    val rollout: Int = 100,
    val urgent: Boolean = false,
    /** Version codes pulled because they have a problem. */
    val withdrawn: List<Long> = emptyList(),
    /** What is new, by language: at most [UpdateLogic.NOTES_MAX_LINES] lines of [UpdateLogic.NOTES_MAX_CHARS] characters. */
    val notes: Map<String, List<String>> = emptyMap(),
    val size: Long = 0,
    val switches: Switches = Switches(),
)

/** Remote switches in the signed manifest; each one defaults to the safe choice when the manifest does not name it. */
data class Switches(
    val installWithoutConfirm: Boolean = false,
    val lockScreenAssistant: Boolean = false,
    val lockScreenAssistOffModels: List<String> = emptyList(),
    val taskOutbox: Boolean = true,
    val startCoverRemoval: Boolean = true,
    val saveCopy: Boolean = true,
) {
    fun lockScreenAssistAllowed(model: String): Boolean = lockScreenAssistant && lockScreenAssistOffModels.none { it.equals(model, true) }
}

/** What HQ's `/api/status/busy` says: whether a chat turn or a backup is running. */
data class BusyStatus(val busy: Boolean, val chatTurns: Int, val backup: Boolean)

/** Why an install is waiting, in the order the dialog names them. */
enum class BusyReason { Chat, Backup, Outbox, Download }

/** Which release stream the owner follows: `stable` is what an owner promoted, `main` is every build of main. */
enum class UpdateChannel(val id: String, val tag: String) {
    Stable("stable", "channel-stable"),
    Main("main", "channel-main");

    companion object {
        fun of(id: String?): UpdateChannel = entries.firstOrNull { it.id == id } ?: Stable
    }
}

sealed interface UpdateVerdict {
    data object UpToDate : UpdateVerdict
    data class Available(val manifest: ChannelManifest) : UpdateVerdict
    data class NeedsNewerAndroid(val minSdk: Int) : UpdateVerdict

    /** The installed build was withdrawn; [fix] is the build that replaces it, when this manifest names one the phone can take. */
    data class InstalledWithdrawn(val version: String, val fix: ChannelManifest?) : UpdateVerdict

    /** A build this phone was offered was withdrawn before it was installed. */
    data class Pulled(val version: String) : UpdateVerdict
}

sealed interface UpdateState {
    data object Idle : UpdateState
    data object Checking : UpdateState
    data object UpToDate : UpdateState
    data class Available(val manifest: ChannelManifest) : UpdateState
    data class NeedsNewerAndroid(val minSdk: Int) : UpdateState
    data class Busy(val manifest: ChannelManifest, val reason: BusyReason) : UpdateState
    data class Waiting(val manifest: ChannelManifest, val chats: Int) : UpdateState
    data class InstalledWithdrawn(val version: String, val fix: ChannelManifest?) : UpdateState
    data class Pulled(val version: String) : UpdateState
    data object Downloading : UpdateState
    data object Installing : UpdateState
    data class Failed(val reason: String) : UpdateState
}

/** The rules for the in-app updater that need no Android: reading the manifest, comparing builds, and when to ask. */
object UpdateLogic {
    /** The only place an APK may come from; a manifest that points anywhere else is refused even though its signature is good. */
    private const val RELEASES_URL = "https://github.com/CalvinMagezi/hq/releases/"
    const val APK_URL_PREFIX = RELEASES_URL + "download/"
    const val CHANNEL_FILE = "android-channel.json"
    const val CHECK_EVERY_MS = 24L * 60 * 60 * 1000

    /** A manifest is a few hundred bytes; this stops a wrong or hostile answer from filling memory. */
    const val MANIFEST_MAX_BYTES = 16L * 1024
    const val APK_MAX_BYTES = 200L * 1024 * 1024

    private const val SHA256_HEX_CHARS = 64
    private const val VERSION_MAX_CHARS = 64
    const val NOTES_MAX_LINES = 3
    const val NOTES_MAX_CHARS = 120
    const val BUSY_PATH = "/api/status/busy"
    const val BUSY_POLL_MS = 10_000L

    const val STOPPED = "The install was stopped before it finished. Tap Check for updates, then Install update, and confirm each prompt Android shows, including Play Protect's \"Install anyway\"."
    const val BLOCKED = "Android blocked the update. If Play Protect stopped it, tap Check for updates, then Install update, and choose \"Install anyway\"."
    const val NOT_INSTALLED = "Android did not install the update."

    /** What the screen says after Android's installer answers; a stopped install is never shown as if nothing happened. */
    fun installOutcome(status: Int, message: String?): UpdateState = when (status) {
        PackageInstaller.STATUS_SUCCESS -> UpdateState.UpToDate
        PackageInstaller.STATUS_FAILURE_ABORTED -> UpdateState.Failed(STOPPED)
        PackageInstaller.STATUS_FAILURE_BLOCKED -> UpdateState.Failed(BLOCKED)
        else -> UpdateState.Failed(message?.takeIf { it.isNotBlank() } ?: NOT_INSTALLED)
    }

    /** Stable is "the latest release", which the Promote workflow keeps pointing at `channel-stable`; main is its own prerelease. */
    fun manifestUrl(channel: UpdateChannel): String = when (channel) {
        UpdateChannel.Stable -> RELEASES_URL + "latest/download/" + CHANNEL_FILE
        UpdateChannel.Main -> APK_URL_PREFIX + channel.tag + "/" + CHANNEL_FILE
    }

    /** The manifest, or null when anything in it is missing or unsafe. Call it only on bytes whose signature has been checked. */
    fun parse(text: String): ChannelManifest? = try {
        val o = JSONObject(text)
        val sw = o.optJSONObject("switches")
        val manifest = ChannelManifest(
            version = o.getString("version"),
            versionCode = o.getLong("versionCode"),
            url = o.getString("url"),
            sha256 = o.getString("sha256").lowercase(),
            minSdk = o.getInt("minSdk"),
            seq = o.optLong("seq", 0),
            issuedAt = o.optString("issuedAt", ""),
            rollout = o.optInt("rollout", 100),
            urgent = o.optBoolean("urgent", false),
            withdrawn = longs(o.optJSONArray("withdrawn")),
            notes = parseNotes(o.opt("notes")) ?: return null,
            size = o.optLong("size", 0),
            switches = Switches(
                installWithoutConfirm = sw?.optBoolean("installWithoutConfirm", false) ?: false,
                lockScreenAssistant = sw?.optBoolean("lockScreenAssistant", false) ?: false,
                lockScreenAssistOffModels = strings(sw?.optJSONArray("lockScreenAssistOffModels")),
                taskOutbox = sw?.optBoolean("taskOutbox", true) ?: true,
                startCoverRemoval = sw?.optBoolean("startCoverRemoval", true) ?: true,
                saveCopy = sw?.optBoolean("saveCopy", true) ?: true,
            ),
        )
        manifest.takeIf { valid(it) }
    } catch (_: JSONException) {
        null
    }

    private fun longs(a: org.json.JSONArray?): List<Long> = List(a?.length() ?: 0) { a!!.getLong(it) }
    private fun strings(a: org.json.JSONArray?): List<String> = List(a?.length() ?: 0) { a!!.getString(it) }

    /** Notes as a plain line (English) or by language; null when there are more than three lines or one is over 120 characters. */
    private fun parseNotes(v: Any?): Map<String, List<String>>? {
        val out = LinkedHashMap<String, List<String>>()
        when (v) {
            null, JSONObject.NULL -> return out
            is String -> out["en"] = listOf(v)
            is JSONObject -> for (lang in v.keys()) {
                val a = v.optJSONArray(lang)
                out[lang] = if (a != null) strings(a) else listOf(v.getString(lang))
            }
            else -> return null
        }
        return out.takeIf { m -> m.values.all { lines -> lines.size <= NOTES_MAX_LINES && lines.all { it.length <= NOTES_MAX_CHARS } } }
    }

    /** The notes for [lang], or the English ones. */
    fun notesFor(m: ChannelManifest, lang: String): List<String> = m.notes[lang] ?: m.notes["en"] ?: emptyList()

    /** True unless this manifest's seq is older than the highest one seen: a replayed old manifest must not undo a newer one. */
    fun seqOk(m: ChannelManifest, highestSeen: Long): Boolean = m.seq >= highestSeen

    private fun valid(m: ChannelManifest): Boolean =
        m.rollout in 0..100 && m.version.isNotBlank() && m.version.length <= VERSION_MAX_CHARS && m.version.none(Char::isWhitespace) &&
            m.versionCode > 0 && m.minSdk > 0 &&
            m.sha256.length == SHA256_HEX_CHARS && m.sha256.all { it in '0'..'9' || it in 'a'..'f' } &&
            m.url.startsWith(APK_URL_PREFIX) && m.url.none { it.isWhitespace() } && m.url.endsWith(".apk")

    /** A build is newer only by its version code: Android refuses to install anything that is not, and a version name proves nothing. */
    fun verdict(
        manifest: ChannelManifest, installedCode: Long, sdk: Int,
        bucket: Int = 0, ignoreRollout: Boolean = true, wasOffered: Boolean = false,
    ): UpdateVerdict {
        val candidate = manifest.versionCode
        if (installedCode in manifest.withdrawn) {
            val fix = manifest.takeIf { candidate > installedCode && candidate !in it.withdrawn && sdk >= it.minSdk }
            return UpdateVerdict.InstalledWithdrawn(installedCode.toString(), fix)
        }
        return when {
            candidate <= installedCode -> UpdateVerdict.UpToDate
            candidate in manifest.withdrawn -> if (wasOffered) UpdateVerdict.Pulled(manifest.version) else UpdateVerdict.UpToDate
            sdk < manifest.minSdk -> UpdateVerdict.NeedsNewerAndroid(manifest.minSdk)
            !ignoreRollout && bucket >= manifest.rollout -> UpdateVerdict.UpToDate
            else -> UpdateVerdict.Available(manifest)
        }
    }

    /** HQ's answer to the busy question, or null when it is not JSON of that shape. */
    fun parseBusy(text: String): BusyStatus? = try {
        val o = JSONObject(text)
        BusyStatus(o.getBoolean("busy"), o.optInt("chat_turns", 0), o.optBoolean("backup", false))
    } catch (_: JSONException) {
        null
    }

    /** The first thing that makes an install wait, in the order Chat, Backup, Outbox, Download; null when HQ is free. HQ not answering counts as free. */
    fun busyReason(busy: BusyStatus?, outboxWaiting: Boolean, downloading: Boolean): BusyReason? = when {
        busy != null && busy.chatTurns > 0 -> BusyReason.Chat
        busy != null && busy.backup -> BusyReason.Backup
        busy != null && outboxWaiting -> BusyReason.Outbox
        downloading -> BusyReason.Download
        else -> null
    }

    /** True when it has been a day since the last check, or the clock moved back past it. */
    fun due(lastCheckMs: Long, nowMs: Long): Boolean = lastCheckMs <= 0 || nowMs < lastCheckMs || nowMs - lastCheckMs >= CHECK_EVERY_MS

    /** The Android release name for an API level, because owners know "Android 10", not "29". */
    fun androidName(sdk: Int): String = when (sdk) {
        26, 27 -> "Android 8"
        28 -> "Android 9"
        29 -> "Android 10"
        30 -> "Android 11"
        31, 32 -> "Android 12"
        33 -> "Android 13"
        34 -> "Android 14"
        35 -> "Android 15"
        36 -> "Android 16"
        else -> "a newer Android"
    }

    /** The line Settings shows for a state; failures say what to do, not what the code did. */
    fun words(state: UpdateState): String = when (state) {
        UpdateState.Idle -> "Not checked yet"
        UpdateState.Checking -> "Checking\u2026"
        UpdateState.UpToDate -> "Up to date"
        is UpdateState.Available -> "Version ${state.manifest.version} is ready"
        is UpdateState.NeedsNewerAndroid -> "A newer version needs ${androidName(state.minSdk)} or newer."
        // The new states are worded from the string catalogue, which needs a Context: see wordsFor.
        is UpdateState.Busy, is UpdateState.Waiting, is UpdateState.InstalledWithdrawn, is UpdateState.Pulled -> ""
        UpdateState.Downloading -> "Downloading\u2026"
        UpdateState.Installing -> "Waiting for you to confirm the install"
        is UpdateState.Failed -> state.reason
    }

    /** [words] for every state, with the catalogue's text for the ones that wait, are withdrawn or were pulled. */
    fun wordsFor(context: android.content.Context, state: UpdateState): String {
        val r = context.resources
        return when (state) {
            is UpdateState.Busy -> context.getString(
                when (state.reason) {
                    BusyReason.Chat -> example.hq.spike.R.string.update_busy_chat
                    BusyReason.Backup -> example.hq.spike.R.string.update_busy_backup
                    BusyReason.Outbox -> example.hq.spike.R.string.update_busy_outbox
                    BusyReason.Download -> example.hq.spike.R.string.update_busy_download
                },
            )
            is UpdateState.Waiting -> if (state.chats > 0) r.getQuantityString(example.hq.spike.R.plurals.update_waiting_chats, state.chats, state.chats.toString()) else context.getString(example.hq.spike.R.string.update_waiting_other)
            is UpdateState.InstalledWithdrawn -> context.getString(example.hq.spike.R.string.update_withdrawn_title, state.version)
            is UpdateState.Pulled -> context.getString(example.hq.spike.R.string.update_pulled_body, state.version)
            else -> words(state)
        }
    }
}
