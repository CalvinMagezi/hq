package example.hq.spike.update

import android.content.pm.PackageInstaller
import org.json.JSONException
import org.json.JSONObject

/** What the signed channel manifest (`android-channel.json`) says the newest APK is. */
data class ChannelManifest(val version: String, val versionCode: Long, val url: String, val sha256: String, val minSdk: Int)

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
}

sealed interface UpdateState {
    data object Idle : UpdateState
    data object Checking : UpdateState
    data object UpToDate : UpdateState
    data class Available(val manifest: ChannelManifest) : UpdateState
    data class NeedsNewerAndroid(val minSdk: Int) : UpdateState
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
        val manifest = ChannelManifest(
            version = o.getString("version"),
            versionCode = o.getLong("versionCode"),
            url = o.getString("url"),
            sha256 = o.getString("sha256").lowercase(),
            minSdk = o.getInt("minSdk"),
        )
        manifest.takeIf { valid(it) }
    } catch (_: JSONException) {
        null
    }

    private fun valid(m: ChannelManifest): Boolean =
        m.version.isNotBlank() && m.version.length <= VERSION_MAX_CHARS && m.version.none(Char::isWhitespace) &&
            m.versionCode > 0 && m.minSdk > 0 &&
            m.sha256.length == SHA256_HEX_CHARS && m.sha256.all { it in '0'..'9' || it in 'a'..'f' } &&
            m.url.startsWith(APK_URL_PREFIX) && m.url.none { it.isWhitespace() } && m.url.endsWith(".apk")

    /** A build is newer only by its version code: Android refuses to install anything that is not, and a version name proves nothing. */
    fun verdict(manifest: ChannelManifest, installedCode: Long, sdk: Int): UpdateVerdict = when {
        manifest.versionCode <= installedCode -> UpdateVerdict.UpToDate
        sdk < manifest.minSdk -> UpdateVerdict.NeedsNewerAndroid(manifest.minSdk)
        else -> UpdateVerdict.Available(manifest)
    }

    /** True when it has been a day since the last check, or the clock moved back past it. */
    fun due(lastCheckMs: Long, nowMs: Long): Boolean = lastCheckMs <= 0 || nowMs < lastCheckMs || nowMs - lastCheckMs >= CHECK_EVERY_MS

    /** The line Settings shows for a state; failures say what to do, not what the code did. */
    fun words(state: UpdateState): String = when (state) {
        UpdateState.Idle -> "Not checked yet"
        UpdateState.Checking -> "Checking\u2026"
        UpdateState.UpToDate -> "Up to date"
        is UpdateState.Available -> "Version ${state.manifest.version} is ready"
        is UpdateState.NeedsNewerAndroid -> "A newer version needs Android ${state.minSdk}"
        UpdateState.Downloading -> "Downloading\u2026"
        UpdateState.Installing -> "Waiting for you to confirm the install"
        is UpdateState.Failed -> state.reason
    }
}
