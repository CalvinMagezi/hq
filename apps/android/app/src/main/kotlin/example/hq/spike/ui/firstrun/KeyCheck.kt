package example.hq.spike.ui.firstrun

import java.io.File
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request

/** The AI services the first run offers. [credential] is the sealed name hq reads; it must end in `_API_KEY`. */
enum class Provider(val label: String, val credential: String, val keyLine: String?, val keyUrl: String?) {
    OpenRouter("OpenRouter", "OPENROUTER_API_KEY", "Get a key at openrouter.ai/keys", "https://openrouter.ai/keys"),
    Anthropic("Anthropic", "ANTHROPIC_API_KEY", "Get a key at console.anthropic.com", "https://console.anthropic.com/settings/keys"),
    Other("Other OpenAI-compatible", "CUSTOM_API_KEY", null, null);

    val needsBaseUrl: Boolean get() = this == Other
}

/** What one key check sends. Holds the key, so it is never printed or logged. */
class CheckRequest(val url: String, val headers: Map<String, String>)

/** The shape checks that run before anything is sealed or sent. They look at the text only. */
object KeyShape {
    /** Null when the key looks like a key: not empty, and no whitespace anywhere. */
    fun keyProblem(value: String): String? = when {
        value.isEmpty() -> FirstRunWords.ERR_KEY_EMPTY
        value.any { it.isWhitespace() || it.isISOControl() } -> FirstRunWords.ERR_KEY_SPACES
        else -> null
    }

    /** Null when the base URL can go into config.yaml: http or https, a host, no login, no quote or backslash. */
    fun baseUrlProblem(raw: String): String? {
        val value = raw.trim()
        if (value.isEmpty()) return FirstRunWords.ERR_URL_EMPTY
        val rest = when {
            value.startsWith("https://", ignoreCase = true) -> value.substring("https://".length)
            value.startsWith("http://", ignoreCase = true) -> value.substring("http://".length)
            else -> return FirstRunWords.ERR_URL_SHAPE
        }
        if (value.any { it.isWhitespace() || it.isISOControl() || it == '"' || it == '\\' }) return FirstRunWords.ERR_URL_SHAPE
        val authority = rest.takeWhile { it != '/' && it != '?' && it != '#' }
        if (authority.isEmpty()) return FirstRunWords.ERR_URL_SHAPE
        if ('@' in authority) return FirstRunWords.ERR_URL_LOGIN
        return null
    }

    fun normalizeBaseUrl(raw: String): String = raw.trim().trimEnd('/')

    /** Null when the optional model name is empty or a single token without quotes. */
    fun modelProblem(raw: String): String? {
        val value = raw.trim()
        return if (value.any { it.isWhitespace() || it.isISOControl() || it == '"' || it == '\\' || it == '#' }) FirstRunWords.ERR_URL_SHAPE else null
    }
}

/** An optional, cheap "is this key accepted" request: one GET to the service, answered with success or the HTTP status. */
object KeyCheck {
    private const val TIMEOUT_SECONDS = 15L
    private val http = OkHttpClient.Builder().callTimeout(TIMEOUT_SECONDS, TimeUnit.SECONDS).build()

    /** OpenRouter's models list is public, so its key endpoint is used: it answers 401 for a bad key. */
    fun request(provider: Provider, key: String, baseUrl: String = ""): CheckRequest = when (provider) {
        Provider.OpenRouter -> CheckRequest("https://openrouter.ai/api/v1/auth/key", mapOf("Authorization" to "Bearer $key"))
        Provider.Anthropic -> CheckRequest("https://api.anthropic.com/v1/models", mapOf("x-api-key" to key, "anthropic-version" to "2023-06-01"))
        Provider.Other -> CheckRequest(KeyShape.normalizeBaseUrl(baseUrl) + "/models", mapOf("Authorization" to "Bearer $key"))
    }

    /** The HTTP status, or null when the service could not be reached. Never logs the request. */
    suspend fun status(request: CheckRequest): Int? = withContext(Dispatchers.IO) {
        try {
            val builder = Request.Builder().url(request.url)
            request.headers.forEach { (k, v) -> builder.header(k, v) }
            http.newCall(builder.build()).execute().use { it.code }
        } catch (_: IOException) {
            null
        } catch (_: IllegalArgumentException) {
            null
        }
    }

    /** The line shown after a check. */
    fun message(status: Int?): String = if (status != null && status in 200..299) FirstRunWords.KEY_OK else FirstRunWords.checkFailed(status)
}

/** The `backends:` block that makes an "Other" service the one HQ talks to. */
object ConfigYaml {
    sealed interface Result {
        class Written(val text: String) : Result
        data object HasBackends : Result
    }

    const val BACKEND_NAME = "custom"
    private val TOP_LEVEL_BACKENDS = Regex("(?m)^backends\\s*:")
    const val MARKER = "# AI service added by the first-run setup"

    fun block(baseUrl: String, model: String, credential: String): String = buildString {
        appendLine(MARKER)
        appendLine("backends:")
        appendLine("  primary: $BACKEND_NAME")
        appendLine("  backends:")
        appendLine("    - name: $BACKEND_NAME")
        appendLine("      kind: openai-compatible")
        appendLine("      endpoint: \"${KeyShape.normalizeBaseUrl(baseUrl)}\"")
        if (model.isNotBlank()) appendLine("      model: \"${model.trim()}\"")
        appendLine("      credential_env: $credential")
    }

    /**
     * Appends the block to the file's text, replacing one this flow wrote earlier (so going back and changing the URL
     * works), or refuses when the file has a top-level `backends:` of its own.
     */
    fun withBackend(existing: String, baseUrl: String, model: String, credential: String): Result {
        val marked = existing.indexOf(MARKER)
        val kept = if (marked >= 0) existing.substring(0, marked) else existing
        if (TOP_LEVEL_BACKENDS.containsMatchIn(kept)) return Result.HasBackends
        val lead = if (kept.isEmpty() || kept.endsWith("\n")) kept else kept + "\n"
        return Result.Written(lead + block(baseUrl, model, credential))
    }

    /** Writes the block to [file] through a temp file and a rename. Returns the reason it did not, or null. */
    fun apply(file: File, baseUrl: String, model: String, credential: String): String? {
        val existing = if (file.exists()) file.readText() else ""
        return when (val r = withBackend(existing, baseUrl, model, credential)) {
            Result.HasBackends -> FirstRunWords.ERR_CONFIG_HAS_BACKENDS
            is Result.Written -> try {
                file.parentFile?.mkdirs()
                val tmp = File(file.parentFile, file.name + ".tmp")
                tmp.writeText(r.text)
                if (!tmp.renameTo(file)) { tmp.delete(); FirstRunWords.ERR_CONFIG_WRITE } else null
            } catch (_: IOException) {
                FirstRunWords.ERR_CONFIG_WRITE
            }
        }
    }
}
