package example.hq.spike.ui.usage

import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import example.hq.spike.ui.theme.HqTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class BalanceRowTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private fun show(chip: BalanceChip?, opened: MutableList<ChipTarget>) = rule.setContent {
        HqTheme { CompositionLocalProvider(LocalBalance provides chip, LocalOnBalance provides { opened += it }) { BalanceRow() } }
    }

    @Test fun showsTheChipsWordsAndOpensSpending() {
        val opened = mutableListOf<ChipTarget>()
        show(BalanceChip("all 85%", "Budget all: \$4.25 of \$5.00 daily", ChipTarget.Spending, ChipTone.Warning), opened)
        rule.onNodeWithText("all 85%").assertIsDisplayed()
        rule.onNodeWithText("all 85%").performClick()
        assertEquals(listOf(ChipTarget.Spending), opened)
    }

    @Test fun opensTheAiServiceForAServiceChip() {
        val opened = mutableListOf<ChipTarget>()
        show(BalanceChip("Copilot 912 left", "GitHub Copilot credits: 912 left", ChipTarget.AiService), opened)
        rule.onNodeWithText("Copilot 912 left").performClick()
        assertEquals(listOf(ChipTarget.AiService), opened)
    }

    @Test fun drawsNothingWithoutABalance() {
        show(null, mutableListOf())
        assertTrue(rule.onAllNodes(androidx.compose.ui.test.hasClickAction()).fetchSemanticsNodes().isEmpty())
    }
}
