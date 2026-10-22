package example.hq.spike.ui.usage

import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToInt
import org.json.JSONArray
import org.json.JSONObject

/** Where the balance chip in the top bar takes the owner when tapped. */
enum class ChipTarget { AiService, Spending }

enum class ChipTone { Normal, Warning, Exceeded }

/** What the top bar shows about spend or balance: [label] on the chip, [title] for its description. */
data class BalanceChip(val label: String, val title: String, val target: ChipTarget, val tone: ChipTone = ChipTone.Normal)

/** What the server said about the AI service in use, spend and balance; a null is a route that did not answer. */
data class UsageSnapshot(
    val settings: JSONObject? = null,
    val providers: JSONObject? = null,
    val copilot: JSONObject? = null,
    val budgets: JSONObject? = null,
)

/** One AI in the order HQ tries them: [position] 1 is the first choice, the rest are what it falls back to. */
data class ChainRow(val backend: String, val provider: String, val model: String, val position: Int) {
    val first: Boolean get() = position == 1
}

/**
 * Which AI service HQ uses, what is left on it and what it has cost. Read from the backends HQ is configured with
 * (their endpoints say whose service each one is), never from which API keys happen to be stored.
 */
object UsageWords {
    const val COPILOT = "GitHub Copilot"
    private const val THOUSAND = 1_000.0
    private const val MILLION = 1_000_000.0
    private const val TRIM_ABOVE = 100.0
    private const val SMALL_USD = 10.0
    private const val CENT = 0.01
    private const val SIX_HOURS = 6

    private fun JSONObject?.arr(name: String): JSONArray? = this?.optJSONArray(name)
    private fun JSONObject?.obj(name: String): JSONObject? = this?.optJSONObject(name)
    private fun JSONArray?.items(): List<JSONObject> = if (this == null) emptyList() else (0 until length()).mapNotNull { optJSONObject(it) }
    private fun JSONObject?.num(name: String): Double? = this?.takeIf { has(name) && !isNull(name) }?.optDouble(name)?.takeIf { it.isFinite() }
    private fun JSONObject?.text(name: String): String? = this?.takeIf { has(name) && !isNull(name) }?.optString(name)?.takeIf { it.isNotBlank() }

    /** Whose service a backend is, from the host it talks to; the kind says it where the host is not enough. */
    fun providerName(host: String?, kind: String?, backend: String = ""): String {
        // The server keeps a port on the host (localhost:11434), which is not part of whose service it is.
        val h = host.orEmpty().lowercase().let { if (it.startsWith("[")) it.substringBefore("]") + "]" else it.substringBefore(":") }
        val k = kind.orEmpty().lowercase()
        return when {
            "githubcopilot.com" in h || "copilot" in k -> COPILOT
            "openrouter" in h || "openrouter" in k -> "OpenRouter"
            h == "api.deepseek.com" -> "DeepSeek"
            // An anthropic-compatible kind with another host is someone else's gateway, named by its host below.
            h == "api.anthropic.com" || ("anthropic" in k && h.isBlank()) -> "Anthropic"
            "googleapis.com" in h -> "Google AI"
            h == "api.openai.com" -> "OpenAI"
            h == "api.groq.com" -> "Groq"
            "cerebras" in h -> "Cerebras"
            "moonshot" in h -> "Moonshot"
            "kimi" in h || "kimi" in k -> "Kimi Code"
            h in LOCAL_HOSTS -> "This phone"
            h.isNotBlank() -> h.removePrefix("api.")
            else -> backend.replaceFirstChar(Char::uppercase).ifBlank { "Not set" }
        }
    }

    private val LOCAL_HOSTS = setOf("localhost", "127.0.0.1", "[::1]", "0.0.0.0")

    /**
     * The AI services in the order HQ tries them, from `backends.entries` of /api/settings. Empty when no order is configured.
     * A backend that is switched off is left out: HQ skips it, and /api/usage/providers (which lists only the ones that are on)
     * is how it is known when the settings do not say.
     */
    fun chain(settings: JSONObject?, providers: JSONObject? = null): List<ChainRow> {
        val b = settings.obj("backends") ?: return emptyList()
        if (!b.optBoolean("configured", true)) return emptyList()
        val enabled = providers.arr("backends")?.let { rows -> rows.items().map { it.optString("backend") }.toSet() }
        val rows = b.arr("entries").items().mapNotNull { e ->
            if (!e.optBoolean("enabled", true) || (enabled != null && enabled.isNotEmpty() && e.optString("name") !in enabled)) return@mapNotNull null
            val position = when {
                e.optBoolean("primary") -> 1
                e.has("fallback_position") && !e.isNull("fallback_position") -> e.optInt("fallback_position") + 2
                else -> return@mapNotNull null
            }
            val name = e.optString("name")
            ChainRow(name, providerName(e.text("endpoint_host"), e.text("kind"), name), e.text("model").orEmpty(), position)
        }
        return rows.sortedBy { it.position }
    }

    /** The AI service that answers first, or null when HQ has no order set. */
    fun primaryProvider(settings: JSONObject?, providers: JSONObject? = null): String? = chain(settings, providers).firstOrNull()?.provider

    /** The distinct services in the chain, first choice first. */
    fun providers(settings: JSONObject?, providers: JSONObject? = null): List<String> = chain(settings, providers).map { it.provider }.distinct()

    /** The provider row HQ built for one backend (status, balance, spend), from /api/usage/providers. */
    fun providerRow(providers: JSONObject?, backend: String): JSONObject? = providers.arr("backends").items().firstOrNull { it.optString("backend") == backend }

    fun statusWords(status: String?): String = when (status) {
        "ok" -> "Working"
        "subscription" -> "Subscription"
        "no_key" -> "No key set"
        "refused" -> "Balance not readable"
        "error" -> "Usage not readable"
        "ledger_only" -> "No balance from the provider"
        "local" -> "Runs on this phone"
        else -> ""
    }

    // ---------------------------------------------------------------- numbers

    /** 12940 as "12.9k", 950 as "950", 1250000 as "1.3M". */
    fun compact(n: Double): String {
        val a = abs(n)
        return when {
            a >= MILLION -> trim(n / MILLION) + "M"
            a >= THOUSAND -> trim(n / THOUSAND) + "k"
            else -> n.roundToInt().toString()
        }
    }

    private fun trim(v: Double): String = if (abs(v) >= TRIM_ABOVE) v.roundToInt().toString() else String.format(Locale.ROOT, "%.1f", v).removeSuffix(".0")

    /** Dollars with cents; small amounts keep three places so a little spend is not "$0.00". */
    fun usd(n: Double?): String {
        if (n == null || !n.isFinite()) return "-"
        if (n != 0.0 && abs(n) < CENT) return "$" + String.format(Locale.ROOT, "%.4f", n)
        val places = if (abs(n) < SMALL_USD) 3 else 2
        return "$" + String.format(Locale.ROOT, "%.${places}f", n).replace(Regex("(\\.\\d\\d)0$"), "$1")
    }

    private fun short(provider: String) = if (provider == COPILOT) "Copilot" else provider

    // ---------------------------------------------------------------- the top bar chip

    /**
     * Budget that needs attention first (it is what is about to block a call), then the balance of the service HQ uses first,
     * then of the next one that has something to show.
     */
    fun chip(s: UsageSnapshot): BalanceChip? = budgetChip(s.budgets) ?: serviceChips(s).firstOrNull()

    private fun budgetChip(b: JSONObject?): BalanceChip? {
        val worst = b.arr("budgets").items().filter { it.optString("state") != "ok" }.maxByOrNull { it.optDouble("pct", 0.0) } ?: return null
        val pct = worst.optDouble("pct", Double.NaN)
        val name = worst.optString("name").ifBlank { "Limit" }
        val period = when (worst.optString("period")) { "day" -> "daily"; "week" -> "weekly"; "month" -> "monthly"; else -> "" }
        val scope = worst.optString("scope").takeIf { it.isNotBlank() && it != "global" }?.let { " on $it" }.orEmpty()
        val title = "Budget $name: ${usd(worst.num("spent_usd"))} of ${usd(worst.num("limit_usd"))} $period$scope".replace("  ", " ")
        val tone = if (worst.optString("state") == "exceeded") ChipTone.Exceeded else ChipTone.Warning
        return BalanceChip("$name ${if (pct.isFinite()) "${pct.roundToInt()}%" else "-"}", title, ChipTarget.Spending, tone)
    }

    /** One chip candidate per service, in the order HQ tries them; the order of the web app's chip when no order is set. */
    private fun serviceChips(s: UsageSnapshot): List<BalanceChip> {
        val chain = chain(s.settings, s.providers)
        if (chain.isEmpty()) return listOfNotNull(copilotChip(s.copilot)) + s.providers.arr("backends").items().mapNotNull { r -> providerChip(r.optString("backend"), r) }
        return chain.distinctBy { it.provider }.mapNotNull { row -> if (row.provider == COPILOT) copilotChip(s.copilot) else providerChip(row.provider, providerRow(s.providers, row.backend)) }
    }

    /** Credits left on the GitHub Copilot plan, from /api/copilot-usage. Null when the plan is not metered or unreadable. */
    fun copilotChip(c: JSONObject?): BalanceChip? {
        if (c == null || !c.optBoolean("active")) return null
        val q = c.obj("quota") ?: return null
        // A plan that is not metered has a quota and no burn: "0 of 0" is no balance, so there is no chip for it.
        if (c.obj("burn") == null) return null
        val left = q.num("remaining") ?: return null
        val total = q.num("entitlement")
        val rate = c.obj("burn").arr("windows").items().firstOrNull { it.optInt("window_hours") == SIX_HOURS }?.num("per_hour")
        val title = "$COPILOT credits: ${compact(left)}${total?.let { " of ${compact(it)}" }.orEmpty()} left" + (rate?.let { ", ${compact(it)}/h over ${SIX_HOURS}h" } ?: "")
        return BalanceChip("Copilot ${compact(left)} left", title, ChipTarget.AiService)
    }

    /** Money in the currency the provider reports it in: dollars read as "$4.20", any other currency keeps its code. */
    fun money(amount: Double, currency: String?): String =
        if (currency.isNullOrBlank() || currency.equals("USD", ignoreCase = true)) usd(amount) else String.format(Locale.ROOT, "%.2f %s", amount, currency.uppercase(Locale.ROOT))

    /** What is left on a key with a spend cap or on the account's balance, as the provider says it; null when it says neither. */
    fun leftText(reading: JSONObject?): String? {
        reading.num("limit_remaining")?.let { return usd(it) }
        val b = reading.obj("balance") ?: return null
        return b.num("amount")?.let { money(it, b.text("currency")) }
    }

    /** What the service cost today: the provider's figure when it gives one, else HQ's own record when that is more than nothing. */
    fun todayAmount(row: JSONObject?): Double? = row.obj("provider").obj("spend").num("today") ?: row.obj("ledger").num("today")?.takeIf { it > 0 }

    /** What is left on a service that has a balance or a cap, else what it cost today. */
    private fun providerChip(provider: String, row: JSONObject?): BalanceChip? {
        val left = leftText(row.obj("provider"))
        val today = todayAmount(row)
        return when {
            left != null -> BalanceChip("${short(provider)} $left left", "$provider: $left left", ChipTarget.AiService)
            today != null -> BalanceChip("${short(provider)} ${usd(today)} today", "$provider: ${usd(today)} today", ChipTarget.AiService)
            else -> null
        }
    }
}
