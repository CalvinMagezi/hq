package example.hq.spike.ui.tasks

import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import example.hq.spike.data.HqClient.TaskItem
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
class SubTasksSemanticsTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
    private fun t(id: String, status: String, parent: String? = null) =
        TaskItem(id, id, "Task $id", "", status, null, null, null, emptyList(), emptyList(), parent, 0, 0, null, null, "")

    @Test fun theHeaderMergesIntoOneNodeSayingHowManyAreDone() {
        val d = TaskDetail(t("a", "to_do"), subtasks = listOf(t("b", "complete", "a"), t("c", "to_do", "a")))
        rule.setContent { HqTheme { SubTasks(d, TasksActions()) } }
        val text = { n: androidx.compose.ui.semantics.SemanticsNode -> n.config.getOrNull(SemanticsProperties.Text)?.joinToString { it.text }.orEmpty() }
        rule.onAllNodes(SemanticsMatcher("merged header") { text(it).contains("SUB-TASKS") && text(it).contains("1\u00a0of\u00a02") }).assertCountEquals(1)
    }
}
