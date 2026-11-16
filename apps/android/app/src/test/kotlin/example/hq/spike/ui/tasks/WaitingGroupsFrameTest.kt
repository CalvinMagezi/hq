package example.hq.spike.ui.tasks

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.ui.theme.HqTheme
import java.time.LocalDate
import java.time.LocalDateTime
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w390dp-h800dp-xxhdpi")
class WaitingGroupsFrameTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val today = LocalDate.of(2026, 11, 16)
    private val now = LocalDateTime.of(2026, 11, 16, 12, 0, 0)

    private fun layoutOf(text: String): TextLayoutResult {
        val results = mutableListOf<TextLayoutResult>()
        rule.onAllNodes(hasText(text), useUnmergedTree = true).fetchSemanticsNodes()
            .forEach { it.config.getOrNull(SemanticsActions.GetTextLayoutResult)?.action?.invoke(results) }
        return results.first { it.layoutInput.text.text == text }
    }

    private fun cutOff(text: String, widthDp: Int): Boolean {
        val l = layoutOf(text)
        val ellipsized = (0 until l.lineCount).any { l.isLineEllipsized(it) }
        val right = rule.onAllNodes(hasText(text), useUnmergedTree = true).fetchSemanticsNodes()
            .first { it.config.getOrNull(SemanticsActions.GetTextLayoutResult) != null }.boundsInRoot.right
        val spills = right > with(rule.density) { widthDp.dp.toPx() }
        return ellipsized || spills
    }

    private fun oldTask(id: String) = TaskItem(
        id = id,
        displayId = "T-$id",
        title = "Review release",
        description = "",
        status = "ready_for_review",
        priority = "high",
        dueDate = null,
        startDate = null,
        tags = emptyList(),
        assignees = emptyList(),
        parentId = null,
        subtaskCount = 0,
        subtaskDone = 0,
        blockedReason = null,
        readyAt = "2026-10-01 10:00:00",
        updatedAt = "2026-10-01 10:00:00",
        initiativeId = "init-1",
    )

    @Test fun groupHeadingWrapsAtTwoHundredPercentAndAnnouncesNameAndCount() {
        val widthDp = 360
        val group = WaitingGroup(
            key = "waiting-init-1",
            initiativeId = "init-1",
            title = "Mobile Redesign",
            tasks = List(12) { oldTask("t$it") },
            oldReviewable = List(12) { oldTask("t$it") },
            defaultOpen = true,
        )
        rule.setContent {
            val d = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(d.density, 2.0f)) {
                HqTheme {
                    Box(Modifier.width(widthDp.dp)) {
                        WaitingGroupHeader(group, open = true, onToggle = {})
                    }
                }
            }
        }
        rule.waitForIdle()
        assertFalse(cutOff("Mobile Redesign", widthDp))
        assertFalse(cutOff("12", widthDp))
        rule.onAllNodes(
            SemanticsMatcher("heading announces name and count") {
                val hasHeading = it.config.getOrNull(SemanticsProperties.Heading) != null
                val cd = it.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString().orEmpty()
                hasHeading && cd.contains("Mobile Redesign") && cd.contains("12")
            },
        ).assertCountEquals(1)
        rule.onAllNodes(
            SemanticsMatcher("collapse toggle is a separate Button") {
                val role = it.config.getOrNull(SemanticsProperties.Role)
                val cd = it.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString().orEmpty()
                role == Role.Button && cd.contains("Mobile Redesign")
            },
        ).assertCountEquals(1)
    }

    @Test fun oldRowWrapsAtTwoHundredPercentWithoutClipping() {
        val widthDp = 360
        val task = oldTask("1")
        rule.setContent {
            val d = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(d.density, 2.0f)) {
                HqTheme {
                    Box(Modifier.width(widthDp.dp)) {
                        TaskRow(task, today, TasksActions(), now)
                    }
                }
            }
        }
        rule.waitForIdle()
        assertFalse(cutOff("Old", widthDp))
        assertFalse(cutOff(task.title, widthDp))
    }

    @Test fun bulkActionContentDescriptionIncludesCount() {
        val group = WaitingGroup(
            key = "waiting-init-1",
            initiativeId = "init-1",
            title = "Mobile Redesign",
            tasks = List(4) { oldTask("t$it") },
            oldReviewable = List(4) { oldTask("t$it") },
            defaultOpen = true,
        )
        rule.setContent {
            HqTheme {
                MarkOldReviewedRow(group, bulking = false, onClick = {})
            }
        }
        rule.waitForIdle()
        rule.onAllNodes(
            SemanticsMatcher("bulk action has count in contentDescription") {
                val cd = it.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString().orEmpty()
                cd.contains("4") && cd.contains("Mobile Redesign")
            },
        ).assertCountEquals(1)
    }
}
