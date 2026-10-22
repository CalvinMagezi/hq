package example.hq.spike.ui.tasks

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class TaskLinksTest {
    private fun urls(text: String) = TaskLinks.annotated(text).let { a -> a.getLinkAnnotations(0, a.length).map { (it.item as androidx.compose.ui.text.LinkAnnotation.Url).url } }

    @Test fun webAddressesBecomeLinksAndTheTextIsUnchanged() {
        val text = "Deployed at https://dev.example.com/app (signed in). Docs: https://example.com/docs/setup."
        assertEquals(text, TaskLinks.annotated(text).text)
        assertEquals(listOf("https://dev.example.com/app", "https://example.com/docs/setup"), urls(text))
    }

    @Test fun aSentenceMarkAfterAnAddressIsNotPartOfIt() {
        assertEquals(listOf("https://example.com/a"), urls("check (https://example.com/a), then https://example.com/a!").distinct())
        assertEquals(listOf("https://example.com/x?y=1"), urls("see https://example.com/x?y=1;"))
    }

    @Test fun textWithoutAddressesHasNoLinks() {
        assertTrue(urls("Handoff note: Notebooks/Projects/Infra/handoff.md, plan ~/.claude/plans/x.md").isEmpty())
        assertTrue(urls("ftp://example.com and mailto:a@b.c are left alone").isEmpty())
    }
}
