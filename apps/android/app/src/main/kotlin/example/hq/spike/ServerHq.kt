package example.hq.spike

import android.content.Context
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import org.json.JSONObject

/**
 * The owner's HQ on a server, which this phone also asks for what waits on the owner, so a coding agent's question on a
 * server HQ reaches the phone with the same buttons as one from the HQ on the phone. The phone signs in like any other
 * device, with a one-time code from that HQ's Settings, Devices, and keeps the device token it gets sealed under the
 * Keystore key ([SecretStore]). Signing the phone out on that HQ, or Disconnect here, ends it.
 */
object ServerHq {
    /** Inbox ids from the server HQ carry this, so a notification and its answer go back to the HQ that asked. */
    const val ID_PREFIX = "server:"
    private const val PREFS = "server_hq"
    private const val ADDRESS = "address"
    private const val SIGNED_OUT = "signed_out"
    private const val CONNECT_MS = 5_000
    private const val READ_MS = 10_000
    private const val MAX_BODY_BYTES = 1 shl 20
    private const val HTTP_OK = 200
    private const val HTTP_UNAUTHORIZED = 401
    private const val HTTP_NOT_FOUND = 404
    private const val HTTP_TOO_MANY = 429

    data class Server(val address: String, val token: String)

    /** What one call answered: the status, the body (empty for a 304) and the ETag it carried. */
    class Reply(@JvmField val status: Int, @JvmField val body: String, @JvmField val etag: String?)

    /** The server this phone is signed in to, or null when there is none or it signed the phone out. */
    @JvmStatic
    fun current(c: Context): Server? {
        val prefs = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val address = prefs.getString(ADDRESS, null) ?: return null
        if (prefs.getBoolean(SIGNED_OUT, false)) return null
        val token = SecretStore.serverToken(c) ?: return null
        return Server(address, token)
    }

    /** The address kept for the settings page, also while signed out. */
    fun address(c: Context): String? = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(ADDRESS, null)

    fun signedOut(c: Context): Boolean = c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(SIGNED_OUT, false)

    /** The server answered 401: its token was signed out there. The phone stops asking until the owner signs in again. */
    @JvmStatic
    fun markSignedOut(c: Context) {
        c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(SIGNED_OUT, true).apply()
        SecretStore.forgetServerToken(c)
    }

    fun disconnect(c: Context) {
        SecretStore.forgetServerToken(c)
        c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().clear().apply()
        Alerts.resetServer(c, false)
    }

    @JvmStatic
    fun isServer(id: String): Boolean = id.startsWith(ID_PREFIX)

    /** The id the server itself uses for an inbox item this phone marked as its. */
    @JvmStatic
    fun rawId(id: String): String = id.removePrefix(ID_PREFIX)

    /**
     * The address as HQ is reached: https only (the app sends nothing in the clear but to itself), a host and an optional
     * port, no path, query or fragment, and no user name or password in it. A typed name without a scheme gets https.
     * Null when what was typed is none of that.
     */
    @JvmStatic
    fun normalize(typed: String): String? {
        var text = typed.trim().trimEnd('/')
        if (text.isEmpty() || text.any { it.isWhitespace() }) return null
        if (!text.contains("://")) text = "https://$text"
        val uri = try {
            URI(text)
        } catch (e: Exception) {
            return null
        }
        val host = uri.host?.lowercase() ?: return null
        if (uri.scheme?.lowercase() != "https" || uri.rawUserInfo != null) return null
        if (!uri.rawPath.isNullOrEmpty() || uri.rawQuery != null || uri.rawFragment != null) return null
        return if (uri.port == -1) "https://$host" else "https://$host:${uri.port}"
    }

    /**
     * Signs this phone in to the HQ at [typed] with a one-time code from its Settings, Devices, and keeps what it gets.
     * Returns null when it worked, else what to tell the owner. Network work: call it off the main thread.
     */
    fun connect(c: Context, typed: String, code: String): String? {
        val address = normalize(typed) ?: return "Type the address you open your HQ at, such as https://hq.example.com."
        if (code.isBlank()) return "Type the one-time code from your HQ's Settings, Devices."
        val reply = try {
            call(address, "/api/auth/exchange", null, JSONObject().put("code", code.trim()).toString(), null)
        } catch (e: IOException) {
            return "Could not reach $address. Check the address, and that this phone can reach it."
        }
        return when (reply.status) {
            HTTP_OK -> {
                val token = runCatching { JSONObject(reply.body).getString("token") }.getOrNull()
                    ?: return "That HQ answered in a way this app does not understand. Is the address right?"
                SecretStore.keepServerToken(c, token)
                c.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(ADDRESS, address).putBoolean(SIGNED_OUT, false).apply()
                Alerts.resetServer(c, true)
                null
            }
            HTTP_UNAUTHORIZED -> "That code did not work. Codes last ten minutes and work once: get a new one."
            HTTP_TOO_MANY -> "Too many tries. Wait a few minutes, then use a new code."
            HTTP_NOT_FOUND -> "That HQ has no sign-in codes turned on. Check the address."
            else -> "Your HQ answered ${reply.status}. Try again in a moment."
        }
    }

    /**
     * One call to the server: a GET when [body] is null, else a POST of that JSON. [etag] asks for a 304 when nothing
     * changed. Throws only when the server could not be reached; any status comes back in the [Reply].
     */
    @JvmStatic
    @JvmOverloads
    fun call(address: String, path: String, token: String?, body: String?, etag: String?, timeoutMs: Int = READ_MS): Reply {
        val h = URL(address + path).openConnection() as HttpURLConnection
        try {
            h.connectTimeout = minOf(CONNECT_MS, timeoutMs)
            h.readTimeout = timeoutMs
            h.instanceFollowRedirects = false
            h.setRequestProperty("X-HQ-Client", "android")
            token?.let { h.setRequestProperty("Authorization", "Bearer $it") }
            etag?.let { h.setRequestProperty("If-None-Match", it) }
            if (body != null) {
                h.requestMethod = "POST"
                h.doOutput = true
                h.setRequestProperty("Content-Type", "application/json")
                h.outputStream.use { it.write(body.toByteArray()) }
            }
            val status = h.responseCode
            val stream = if (status in 200..299) h.inputStream else h.errorStream
            return Reply(status, stream?.let(::readCapped).orEmpty(), h.getHeaderField("ETag"))
        } finally {
            h.disconnect()
        }
    }

    /** At most [MAX_BODY_BYTES] of a reply, so a wrong address cannot fill memory. */
    private fun readCapped(input: java.io.InputStream): String = input.use { s ->
        val buf = ByteArray(MAX_BODY_BYTES)
        var n = 0
        while (n < buf.size) {
            val r = s.read(buf, n, buf.size - n)
            if (r <= 0) break
            n += r
        }
        String(buf, 0, n, Charsets.UTF_8)
    }
}
