package example.hq.spike.voice

/**
 * The voice reads words from a pronunciation list that has no digits and few acronyms, and silently skips what it
 * does not know. This writes numbers out and spells short capitalised words letter by letter before it speaks.
 */
object SpeechWords {
    private val ACRONYM = Regex("""\b[A-Z]{2,5}s?\b""")
    private val NUMBER = Regex("""(?<![\w.])(\d{1,3}(?:,\d{3})+|\d+)(?:\.(\d+))?(%?)""")
    private val SYMBOLS = mapOf("&" to " and ", "+" to " plus ", "=" to " equals ", "@" to " at ", "°" to " degrees ")
    private val ONES = listOf("zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen")
    private val TENS = listOf("", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety")
    private val SCALES = listOf(1_000_000_000_000L to "trillion", 1_000_000_000L to "billion", 1_000_000L to "million", 1_000L to "thousand")
    private const val HUNDRED = 100L
    private const val TWENTY = 20L
    private const val TEN = 10L
    /** Longer digit runs (phone numbers, ids) are read digit by digit, which is how people say them. */
    private const val LONGEST_NUMBER_DIGITS = 13

    fun forVoice(text: String): String {
        var s = text
        SYMBOLS.forEach { (symbol, word) -> s = s.replace(symbol, word) }
        s = ACRONYM.replace(s) { spell(it.value) }
        s = NUMBER.replace(s) { m -> number(m.groupValues[1], m.groupValues[2], m.groupValues[3]) }
        return s.replace(Regex("""\s+"""), " ").trim()
    }

    /** A plural's "s" is dropped: the list has no "i's", and "a p i" is clearer than a guess. */
    private fun spell(word: String): String = word.removeSuffix("s").lowercase().toList().joinToString(" ")

    private fun number(whole: String, fraction: String, percent: String): String {
        val digits = whole.replace(",", "")
        val main = if (digits.length > LONGEST_NUMBER_DIGITS) digitByDigit(digits) else words(digits.toLong())
        val point = if (fraction.isEmpty()) "" else " point " + digitByDigit(fraction)
        val per = if (percent.isEmpty()) "" else " percent"
        return " $main$point$per "
    }

    private fun digitByDigit(digits: String) = digits.map { ONES[it - '0'] }.joinToString(" ")

    fun words(n: Long): String {
        if (n < TWENTY) return ONES[n.toInt()]
        if (n < HUNDRED) return TENS[(n / TEN).toInt()] + if (n % TEN == 0L) "" else " " + ONES[(n % TEN).toInt()]
        if (n < SCALES.last().first) return ONES[(n / HUNDRED).toInt()] + " hundred" + rest(n % HUNDRED)
        val (scale, name) = SCALES.first { n >= it.first }
        return words(n / scale) + " " + name + rest(n % scale)
    }

    private fun rest(n: Long) = if (n == 0L) "" else " " + words(n)
}
