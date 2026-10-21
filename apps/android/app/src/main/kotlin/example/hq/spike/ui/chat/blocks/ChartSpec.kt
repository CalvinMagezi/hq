package example.hq.spike.ui.chat.blocks

import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.log10
import kotlin.math.pow
import kotlin.math.roundToInt
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

enum class ChartType { Line, Bar, Pie, Donut;
    val isRound: Boolean get() = this == Pie || this == Donut
    val title: String get() = when (this) { Line -> "Line"; Bar -> "Bar"; Pie -> "Pie"; Donut -> "Donut" }
}

data class ChartSeries(val name: String, val values: List<Double>)

data class ChartSpec(
    val type: ChartType,
    val title: String,
    val xLabel: String,
    val yLabel: String,
    val labels: List<String>,
    val series: List<ChartSeries>,
)

sealed interface ChartParse {
    data class Ok(val spec: ChartSpec) : ChartParse
    data class Bad(val reason: String) : ChartParse
}

/**
 * The `chart` block format of docs/CHART_BLOCKS.md, read exactly as apps/hq-web/src/lib/chart.ts reads it, so the
 * web app and the phone accept and refuse the same payloads with the same reasons.
 */
object Charts {
    const val MAX_POINTS = 50
    const val MAX_SLICES = 12
    const val MAX_SERIES = 6
    private const val MAX_TEXT = 120
    private const val MAX_TICK_LABEL = 10
    private const val Y_TICKS = 4
    private const val MAX_TICKS = 12
    /** Far more than any figure a chart shows; keeps the scale and its arithmetic inside what a Double and a Long hold. */
    private const val MAX_MAGNITUDE = 9e16
    private const val MAX_SOURCE_CHARS = 32_000
    private const val MAX_DEPTH = 16
    /** Android's parser also reads single-quoted strings and comments, which the depth count above does not follow, so the brackets are limited outright too. */
    private const val MAX_BRACKETS = 1_000
    private val MERMAID_PIE_ROW = Regex("^\\s*\"([^\"]+)\"\\s*:\\s*(-?\\d+(?:\\.\\d+)?)\\s*$")

    fun parse(source: String): ChartParse {
        if (source.length > MAX_SOURCE_CHARS || source.count { it == '[' || it == '{' } > MAX_BRACKETS || nesting(source) > MAX_DEPTH) return bad("payload is too large")
        val raw = try { JSONTokenerStrict.value(source) } catch (e: JSONException) { return bad("payload is not valid JSON") }
        if (raw !is JSONObject) return bad("payload must be a JSON object")
        val type = ChartType.entries.firstOrNull { it.name.lowercase() == raw.opt("type") } ?: return bad("type must be one of line, bar, pie, donut")
        val rawLabels = raw.opt("labels") as? JSONArray
        if (rawLabels == null || rawLabels.length() == 0) return bad("labels must be a non-empty array")
        val limit = if (type.isRound) MAX_SLICES else MAX_POINTS
        if (rawLabels.length() > limit) return bad("at most $limit labels are supported for ${type.name.lowercase()} charts")
        val labels = (0 until rawLabels.length()).map { text(rawLabels.opt(it)) }
        val series = parseSeries(raw.opt("series"), labels.size)
        if (series is String) return bad(series)
        @Suppress("UNCHECKED_CAST") val parsed = series as List<ChartSeries>
        if (type.isRound) {
            val values = parsed[0].values
            if (values.any { it < 0 }) return bad("pie and donut values must not be negative")
            if (values.sum() <= 0) return bad("pie and donut values must sum to more than zero")
        }
        return ChartParse.Ok(ChartSpec(type, text(raw.opt("title")), text(raw.opt("xLabel")), text(raw.opt("yLabel")), labels, parsed))
    }

    /** A Mermaid `pie` block, which models often write instead of a chart block. */
    fun parseMermaidPie(source: String): ChartParse {
        val lines = source.lines().filter { it.isNotBlank() }
        if (lines.isEmpty() || !Regex("^\\s*pie\\b").containsMatchIn(lines[0])) return bad("not a pie diagram")
        var title = ""
        val labels = mutableListOf<String>()
        val values = mutableListOf<Double>()
        for (line in lines.drop(1)) {
            Regex("^\\s*title\\s+(.+)$").find(line)?.let { title = it.groupValues[1].trim(); continue }
            val row = MERMAID_PIE_ROW.find(line) ?: return bad("unsupported Mermaid pie syntax")
            labels += row.groupValues[1]
            val value = row.groupValues[2].toDouble()
            if (!value.isFinite() || abs(value) >= MAX_MAGNITUDE) return bad("a value is too large")
            values += value
        }
        val payload = JSONObject().put("type", "pie").put("title", title).put("labels", JSONArray(labels))
            .put("series", JSONArray().put(JSONObject().put("name", title.ifEmpty { "Value" }).put("values", JSONArray(values))))
        return parse(payload.toString())
    }

    private fun parseSeries(raw: Any?, pointCount: Int): Any {
        val arr = raw as? JSONArray
        if (arr == null || arr.length() == 0) return "series must be a non-empty array"
        if (arr.length() > MAX_SERIES) return "at most $MAX_SERIES series are supported"
        val out = mutableListOf<ChartSeries>()
        for (i in 0 until arr.length()) {
            val item = arr.opt(i) as? JSONObject
            val values = item?.opt("values") as? JSONArray ?: return "series ${i + 1} needs a values array"
            if (values.length() != pointCount) return "series ${i + 1} needs $pointCount values, one for each label"
            val numbers = (0 until values.length()).map { values.opt(it) }
            if (!numbers.all { it is Number && it.toDouble().isFinite() }) return "series ${i + 1} has a non-numeric value"
            if (numbers.any { abs((it as Number).toDouble()) >= MAX_MAGNITUDE }) return "series ${i + 1} has a value that is too large"
            out += ChartSeries(text(item.opt("name"), "Series ${i + 1}"), numbers.map { (it as Number).toDouble() })
        }
        return out
    }

    private fun text(v: Any?, fallback: String = ""): String = if (v is String || v is Number) numberText(v).take(MAX_TEXT) else fallback

    private fun numberText(v: Any): String = if (v is Number && v.toDouble() == floor(v.toDouble()) && abs(v.toDouble()) < 1e15) v.toLong().toString() else v.toString()

    private fun bad(reason: String) = ChartParse.Bad(reason)

    /** A round step (1, 2 or 5 times a power of ten) that gives about [count] intervals. */
    fun niceStep(range: Double, count: Int): Double {
        val rough = range / count
        val pow = 10.0.pow(floor(log10(rough)))
        val frac = rough / pow
        val nice = when { frac <= 1 -> 1.0; frac <= 2 -> 2.0; frac <= 5 -> 5.0; else -> 10.0 }
        return nice * pow
    }

    data class YScale(val min: Double, val max: Double, val ticks: List<Double>)

    fun yScale(values: List<Double>): YScale {
        val lo = minOf(0.0, values.minOrNull() ?: 0.0)
        val hi = maxOf(0.0, values.maxOrNull() ?: 0.0)
        val step = niceStep(if (hi == lo) 1.0 else hi - lo, Y_TICKS)
        val min = floor(lo / step) * step
        val max = ceil(hi / step) * step
        val ticks = mutableListOf<Double>()
        var v = min
        while (v <= max + step / 2 && ticks.size < MAX_TICKS) { ticks += round2(v); v += step }
        return YScale(min, if (max == min) min + step else max, ticks)
    }

    fun round2(n: Double): Double = Math.round(n * 100) / 100.0

    /** A figure the way it is printed in a tooltip or a table: whole numbers whole, others to two places, never in scientific form. */
    fun plain(n: Double): String =
        if (n == floor(n) && abs(n) < MAX_MAGNITUDE) n.toLong().toString() else java.math.BigDecimal.valueOf(round2(n)).stripTrailingZeros().toPlainString()

    /** How deep the brackets go, ignoring any inside strings; a payload nested past a few levels is not a chart, and parsing it could overflow the stack. */
    private fun nesting(s: String): Int {
        var depth = 0
        var deepest = 0
        var inString = false
        var escaped = false
        for (c in s) {
            when {
                escaped -> escaped = false
                inString && c == '\\' -> escaped = true
                c == '"' -> inString = !inString
                inString -> {}
                c == '{' || c == '[' -> { depth++; if (depth > deepest) deepest = depth }
                c == '}' || c == ']' -> depth--
            }
        }
        return deepest
    }

    /** 1200 as "1.2K", 1500000 as "1.5M": the compact form the web charts print on their axes and in legends. */
    fun compact(n: Double): String {
        val a = abs(n)
        val (div, suffix) = when {
            a >= 1e12 -> 1e12 to "T"; a >= 1e9 -> 1e9 to "B"; a >= 1e6 -> 1e6 to "M"; a >= 1e3 -> 1e3 to "K"; else -> 1.0 to ""
        }
        val scaled = (n / div * 10).roundToInt() / 10.0
        return (if (scaled == floor(scaled)) scaled.toLong().toString() else scaled.toString()) + suffix
    }

    /** An x axis label cut to ten characters, the way the web chart does. */
    fun clip(s: String): String = if (s.length > MAX_TICK_LABEL) s.take(MAX_TICK_LABEL - 1) + "…" else s

    /** Every n-th label is printed so a long axis stays readable. */
    fun labelEvery(count: Int): Int = ceil(count / 8.0).toInt().coerceAtLeast(1)

    fun percent(value: Double, total: Double): Int = (value / total * 100).roundToInt()
}

/** org.json parses `[1,2]` and `{...}` but also accepts loose forms; this keeps to what JSON.parse accepts for the cases that matter. */
private object JSONTokenerStrict {
    fun value(s: String): Any {
        val t = s.trim()
        if (t.isEmpty()) throw JSONException("empty")
        return when (t[0]) {
            '{' -> JSONObject(t)
            '[' -> JSONArray(t)
            else -> throw JSONException("not an object or array")
        }
    }
}
