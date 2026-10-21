package example.hq.spike.ui.chat.blocks

import example.hq.spike.data.ToolStep
import java.time.LocalDate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class StepCardsTest {
    private val today = LocalDate.of(2026, 10, 21)
    private fun step(name: String, args: String) = ToolStep("id-$name-${args.hashCode()}", name, args, null, null)

    @Test fun aSavedNoteBecomesACardAndIsNotAlsoAChip() {
        val cards = StepCards.of(
            listOf(
                step("vault_read", """{"path":"Notebooks/Projects/Agent-HQ/Roadmap.md"}"""),
                step("vault_write_note", """{"path":"Notebooks/Projects/Agent-HQ/Samsung battery steps.md"}"""),
                step("vault_read", """{"path":"Notebooks/Projects/Agent-HQ/Samsung battery steps.md"}"""),
            ),
            today,
        )
        assertEquals(listOf("Samsung battery steps"), cards.written.map { it.title })
        assertEquals("Projects › Agent-HQ", cards.written.single().trail)
        assertEquals(listOf("Notebooks/Projects/Agent-HQ/Roadmap.md"), cards.read)
    }

    @Test fun aPathThatLeavesTheVaultIsIgnored() {
        val cards = StepCards.of(listOf(step("vault_read", """{"path":"../secrets.md"}"""), step("vault_read", """{"path":"/etc/hosts"}""")), today)
        assertTrue(cards.isEmpty)
    }

    @Test fun aTaskCardSaysWhenItIsDueAndHowUrgent() {
        fun meta(args: String) = StepCards.of(listOf(step("task_create", args)), today).tasks.single().meta
        assertEquals("Due tomorrow · High", meta("""{"title":"Check","due_date":"2026-10-22","priority":"high"}"""))
        assertEquals("Due today", meta("""{"title":"Check","due_date":"2026-10-21","priority":"normal"}"""))
        assertEquals("Urgent", meta("""{"title":"Check","priority":"urgent"}"""))
        assertEquals("In your tasks", meta("""{"title":"Check"}"""))
    }

    @Test fun brokenArgumentsAreSkippedNotFatal() {
        assertTrue(StepCards.of(listOf(ToolStep("a", "vault_read", "{not json", null, null), ToolStep("b", "task_create", null, null, null)), today).isEmpty)
    }
}
