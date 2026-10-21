package example.hq.spike.ui.chat.blocks

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The cases of apps/hq-web/src/lib/chart.test.ts that concern parsing, limits and scaling. */
class ChartSpecTest {
    private fun payload(vararg over: Pair<String, Any?>): String {
        val base = JSONObject()
            .put("type", "line").put("title", "Sales").put("xLabel", "Month").put("yLabel", "Units")
            .put("labels", JSONArray(listOf("Jan", "Feb", "Mar")))
            .put("series", JSONArray().put(JSONObject().put("name", "North").put("values", JSONArray(listOf(1, 4, 2))))
                .put(JSONObject().put("name", "South").put("values", JSONArray(listOf(3, 2, 5)))))
        over.forEach { (k, v) -> base.put(k, v) }
        return base.toString()
    }

    private fun series(name: String, vararg v: Any?) = JSONArray().put(JSONObject().put("name", name).put("values", JSONArray(v.toList())))

    private fun reason(src: String) = (Charts.parse(src) as ChartParse.Bad).reason

    @Test fun aValueTooLargeToDrawIsRefusedInsteadOfHangingTheScale() {
        assertEquals("series 1 has a value that is too large", reason(payload("series" to series("s", 1.7e308, 1.0, 2.0))))
        assertEquals("series 1 has a value that is too large", reason(payload("series" to series("s", -1.7e308, 1.0, 2.0))))
    }

    @Test fun theScaleAlwaysEndsWithFewTicks() {
        for (top in listOf(1.0, 9.9e14, 0.000001, 123456789012.0)) assertTrue(Charts.yScale(listOf(0.0, top)).ticks.size <= 13)
    }

    @Test fun aPayloadNestedFarTooDeepIsRefusedNotParsed() {
        assertEquals("payload is too large", reason("[".repeat(50_000)))
        assertEquals("payload is too large", reason("{\"a\":".repeat(40) + "1" + "}".repeat(40)))
        assertEquals("payload is too large", reason("x".repeat(40_000).let { "[\"$it\"]" }))
    }

    @Test fun singleQuotesAndCommentsCannotHideDepthFromTheCheck() {
        // Android's parser reads 'a"b' as one string, so a count that follows only double quotes would lose track of what is nested.
        val sneaky = "{'a':'\"','b':" + "[".repeat(5_000) + "1" + "]".repeat(5_000) + "}"
        assertEquals("payload is too large", reason(sneaky))
    }

    @Test fun figuresArePrintedPlainNeverInScientificForm() {
        assertEquals("30000000.5", Charts.plain(30_000_000.5))
        assertEquals("2.5", Charts.plain(2.5))
        assertEquals("-0.25", Charts.plain(-0.25))
        assertEquals("1200", Charts.plain(1200.0))
        assertEquals("25000000000000000", Charts.plain(2.5e16))
    }

    @Test fun figuresInTheTrillionsAndBeyondAreStillAccepted() {
        assertTrue(Charts.parse(payload("series" to series("s", 2.5e16, 1.0, 2.0))) is ChartParse.Ok)
    }

    @Test fun bracketsInsideTextDoNotCountAsDepth() {
        val labels = JSONArray(listOf("[[[[[[[[[[[[[[[[[[[[", "{{{{{{{{{{{{{{{{{{{{{", "Mar"))
        assertTrue(Charts.parse(payload("labels" to labels)) is ChartParse.Ok)
    }

    @Test fun roundingKeepsFiguresInTheTensOfMillions() {
        assertEquals(30_000_000.0, Charts.round2(30_000_000.0), 0.0)
        assertEquals(30_000_000.5, Charts.round2(30_000_000.5), 0.0)
        assertEquals(1.24, Charts.round2(1.2449), 0.0)
        val ticks = Charts.yScale(listOf(25_000_000.0, 40_000_000.0)).ticks
        assertEquals(ticks.sorted().distinct(), ticks)
        assertEquals(40_000_000.0, ticks.last(), 0.0)
    }

    @Test fun aMermaidPieWithAnEnormousNumberIsRefused() {
        val src = "pie title Spend\n\"a\": ${"9".repeat(400)}\n\"b\": 1"
        assertEquals("a value is too large", (Charts.parseMermaidPie(src) as ChartParse.Bad).reason)
    }

    @Test fun aValidChartParses() {
        val spec = (Charts.parse(payload()) as ChartParse.Ok).spec
        assertEquals(ChartType.Line, spec.type)
        assertEquals(listOf("Jan", "Feb", "Mar"), spec.labels)
        assertEquals(2, spec.series.size)
        assertEquals("Month", spec.xLabel)
    }

    @Test fun invalidPayloadsGiveAReasonInsteadOfThrowing() {
        val cases = listOf(
            "" to "not valid JSON", "{oops" to "not valid JSON", "[1,2]" to "JSON object",
            payload("type" to "radar") to "type must be", payload("labels" to JSONArray()) to "labels", payload("labels" to "abc") to "labels",
            payload("series" to JSONArray()) to "series", payload("series" to series("x", 1, 2)) to "needs 3 values",
            payload("series" to series("x", 1, "two", 3)) to "non-numeric", payload("series" to series("x", 1, JSONObject.NULL, 3)) to "non-numeric",
            payload("series" to JSONArray().put(JSONObject().put("name", "x"))) to "values array",
            payload("type" to "pie", "series" to series("x", 1, -1, 3)) to "negative",
            payload("type" to "donut", "series" to series("x", 0, 0, 0)) to "sum to more than zero",
        )
        for ((src, expected) in cases) assertTrue("'$src' -> ${reason(src)}", reason(src).contains(expected))
    }

    @Test fun sizeLimitsAreEnforced() {
        val many = List(Charts.MAX_POINTS + 1) { it.toString() }
        assertTrue(Charts.parse(payload("labels" to JSONArray(many), "series" to JSONArray().put(JSONObject().put("name", "a").put("values", JSONArray(many.map { 1 }))))) is ChartParse.Bad)
        val slices = List(Charts.MAX_SLICES + 1) { it.toString() }
        assertTrue(Charts.parse(payload("type" to "pie", "labels" to JSONArray(slices), "series" to JSONArray().put(JSONObject().put("name", "a").put("values", JSONArray(slices.map { 1 }))))) is ChartParse.Bad)
        val tooMany = JSONArray().also { a -> repeat(Charts.MAX_SERIES + 1) { a.put(JSONObject().put("name", "s").put("values", JSONArray(listOf(1, 2, 3)))) } }
        assertTrue(Charts.parse(payload("series" to tooMany)) is ChartParse.Bad)
    }

    @Test fun aChartWithoutTitleOrAxisLabelsStillParses() {
        val ok = Charts.parse("""{"type":"bar","labels":["a"],"series":[{"name":"s","values":[2]}]}""") as ChartParse.Ok
        assertEquals("", ok.spec.title)
    }

    @Test fun negativeValuesKeepAZeroBaselineInsideTheScale() {
        val s = Charts.yScale(listOf(-5.0, 3.0))
        assertTrue(s.min <= -5.0 && s.max >= 3.0)
        assertTrue(s.ticks.contains(0.0))
    }

    @Test fun anAllZeroSeriesStillMakesAValidScale() {
        val s = Charts.yScale(listOf(0.0, 0.0))
        assertTrue(s.max > s.min)
        assertEquals(0.5, Charts.niceStep(1.0, 4), 0.0)
    }

    @Test fun theDesignSampleScalesTo0_30WithFourGridlines() {
        val s = Charts.yScale(listOf(18.0, 24.0, 21.0, 30.0, 27.0))
        assertEquals(listOf(0.0, 10.0, 20.0, 30.0), s.ticks)
    }

    @Test fun mermaidPieBecomesAPieChart() {
        val ok = Charts.parseMermaidPie("pie showData\n    title This week outflows (UGX)\n    \"School fees\" : 2775000\n    \"Loans\" : 1502387\n") as ChartParse.Ok
        assertEquals(ChartType.Pie, ok.spec.type)
        assertEquals(listOf("School fees", "Loans"), ok.spec.labels)
        assertEquals(listOf(2775000.0, 1502387.0), ok.spec.series[0].values)
        assertEquals("This week outflows (UGX)", ok.spec.title)
    }

    @Test fun otherMermaidAndUnknownSyntaxAreLeftAlone() {
        assertTrue(Charts.parseMermaidPie("graph TD\n A-->B") is ChartParse.Bad)
        assertTrue(Charts.parseMermaidPie("pie\n \"a\" : 1\n click a callback") is ChartParse.Bad)
        assertTrue(Charts.parseMermaidPie("pie\n") is ChartParse.Bad)
    }

    @Test fun compactAndClipMatchTheWebLabels() {
        assertEquals("1.2K", Charts.compact(1234.0))
        assertEquals("30", Charts.compact(30.0))
        assertEquals("1.5M", Charts.compact(1_500_000.0))
        assertEquals("Reading n…", Charts.clip("Reading notes"))
        assertEquals(2, Charts.labelEvery(16))
        assertEquals(42, Charts.percent(16.0, 38.0))
    }
}
