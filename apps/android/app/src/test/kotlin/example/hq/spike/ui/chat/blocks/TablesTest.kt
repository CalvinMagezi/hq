package example.hq.spike.ui.chat.blocks

import androidx.compose.ui.unit.dp
import org.junit.Assert.assertEquals
import org.junit.Test

class TablesTest {
    private val table = Block.Table(listOf("Run", "Time"), listOf(Align.Start, Align.End), listOf(listOf("Run 1", "2 h"), listOf("Run 2", "41 m")))

    @Test fun copyingATableGivesTabSeparatedRows() {
        assertEquals("Run\tTime\nRun 1\t2 h\nRun 2\t41 m", table.toTsv())
    }

    @Test fun theNumbersUnderALineChartHaveOneColumnPerSeries() {
        val spec = ChartSpec(ChartType.Line, "t", "Week", "", listOf("W1", "W2"), listOf(ChartSeries("Web", listOf(12.0, 18.0)), ChartSeries("Mobile", listOf(5.5, 9.0))))
        val t = spec.numbersTable()
        assertEquals(listOf("Week", "Web", "Mobile"), t.headers)
        assertEquals(listOf(listOf("W1", "12", "5.5"), listOf("W2", "18", "9")), t.rows)
    }

    @Test fun theNumbersUnderADonutGiveValueAndShare() {
        val spec = ChartSpec(ChartType.Donut, "t", "", "min", listOf("A", "B"), listOf(ChartSeries("m", listOf(3.0, 1.0))))
        assertEquals(listOf(listOf("A", "3", "75%"), listOf("B", "1", "25%")), spec.numbersTable().rows)
    }

    @Test fun aNarrowTableStretchesItsLastColumnToTheEdge() {
        assertEquals(listOf(104.dp, 241.dp), filled(listOf(104.dp, 200.dp), 345.dp))
    }

    @Test fun aTableThatAlreadyFillsTheBoxIsLeftAlone() {
        assertEquals(listOf(104.dp, 300.dp), filled(listOf(104.dp, 300.dp), 345.dp))
    }
}
