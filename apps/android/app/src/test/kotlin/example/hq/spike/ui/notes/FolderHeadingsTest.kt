package example.hq.spike.ui.notes

import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import example.hq.spike.ui.theme.HqTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class FolderHeadingsTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    @Test fun theFolderSectionHeadingsAreHeadingsWithTheirCounts() {
        rule.setContent { HqTheme { SectionLabel("Folders", 3); SectionLabel("Notes, A to Z", 12) } }
        rule.onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading)).assertCountEquals(2)
        val text = { n: androidx.compose.ui.semantics.SemanticsNode -> n.config.getOrNull(SemanticsProperties.Text)?.joinToString { it.text }.orEmpty() }
        rule.onAllNodes(SemanticsMatcher("folders heading with count") { text(it).contains("FOLDERS") && text(it).contains("3") }).assertCountEquals(1)
        rule.onAllNodes(SemanticsMatcher("notes heading with count") { text(it).contains("NOTES, A TO Z") && text(it).contains("12") }).assertCountEquals(1)
    }
}
