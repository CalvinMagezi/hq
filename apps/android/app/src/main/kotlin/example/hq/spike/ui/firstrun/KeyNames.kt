package example.hq.spike.ui.firstrun

/** A stored key's name as the person reads it: the service it opens, not the name HQ looks for. */
object KeyNames {
    private val KNOWN = mapOf(
        "OPENROUTER_API_KEY" to "OpenRouter",
        "ANTHROPIC_API_KEY" to "Anthropic",
        "OPENAI_API_KEY" to "OpenAI",
        "GOOGLE_API_KEY" to "Google",
        "GEMINI_API_KEY" to "Google",
        "BRAVE_API_KEY" to "Brave Search",
        "TELEGRAM_BOT_TOKEN" to "Telegram",
        "DISCORD_BOT_TOKEN" to "Discord",
        "CUSTOM_API_KEY" to "Another AI service",
    )

    /** An unknown name shows as typed, so nothing is renamed wrongly. */
    fun display(name: String): String = KNOWN[name] ?: Provider.entries.firstOrNull { it.credential == name }?.label ?: name
}
