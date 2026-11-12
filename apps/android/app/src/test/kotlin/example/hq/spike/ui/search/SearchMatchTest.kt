package example.hq.spike.ui.search

import example.hq.spike.data.HqClient.TaskItem
import example.hq.spike.ui.chat.ChatRow
import example.hq.spike.ui.chat.Group
import org.junit.Assert.assertEquals
import org.junit.Test

class SearchMatchTest {
    private fun task(id: String, title: String, desc: String = "", priority: String? = null) =
        TaskItem(id, "P-$id", title, desc, "to_do", priority, null, null, emptyList(), emptyList(), null, 0, 0, null, null, "2026-10-20 10:00:00")

    @Test fun everyWordHasToBeThereInAnyOrderAndCaseIsIgnored() {
        assertEquals(true, SearchMatch.hits("Workstream C: a native view", "NATIVE workstream"))
        assertEquals(false, SearchMatch.hits("Workstream C: a native view", "native backup"))
        assertEquals(false, SearchMatch.hits("anything", "  "))
    }

    @Test fun tasksMatchOnNumberTitleAndNotesMostUrgentFirst() {
        val a = task("1", "Soak test", priority = null)
        val b = task("2", "Backup", desc = "the soak run needs it", priority = "high")
        val c = task("3", "Unrelated")
        assertEquals(listOf("2", "1"), SearchMatch.tasks(listOf(a, b, c), "soak").map { it.id })
        assertEquals(listOf("3"), SearchMatch.tasks(listOf(a, b, c), "p-3").map { it.id })
    }

    @Test fun chatsMatchOnTitleAndPreview() {
        val rows = listOf(ChatRow("a", "Native view", "3m ago · Nothing is blocking it", Group.Today, false, false), ChatRow("b", "Other", "", Group.Today, false, false))
        assertEquals(listOf("a"), SearchMatch.chats(rows, "blocking").map { it.id })
    }

    @Test fun recentSearchesKeepTheNewestOnceAndNotTheEmptyOne() {
        assertEquals(listOf("b", "a"), SearchMatch.remember(listOf("a"), "b"))
        assertEquals(listOf("A", "b"), SearchMatch.remember(listOf("b", "a"), " A "))
        assertEquals(listOf("a"), SearchMatch.remember(listOf("a"), "  "))
        assertEquals(SearchMatch.RECENT_KEPT, SearchMatch.remember((1..20).map { "q$it" }, "new").size)
    }

    @Test fun aSearchWithALineBreakIsKeptAsOneLineAndNeverTwiceUnderOneKey() {
        assertEquals(listOf("bar foo", "foo"), SearchMatch.remember(listOf("foo"), "bar\nfoo"))
        assertEquals(listOf("foo"), SearchMatch.remember(listOf("foo"), "  foo \n "))
        assertEquals(true, SearchMatch.hits("alpha beta", "alpha\n beta"))
    }
}

class TagChipsTest {
    @Test fun `the active tag comes first and each tag once`() {
        assertEquals(listOf("work", "idea", "android"), SearchMatch.tagChips(listOf("idea", "Work", "android", "idea"), "work"))
    }

    @Test fun `at most eight tag chips and none blank`() {
        assertEquals(SearchMatch.MAX_TAG_CHIPS, SearchMatch.tagChips((1..20).map { "t$it" }, null).size)
        assertEquals(listOf("a"), SearchMatch.tagChips(listOf("#a", "", "#"), null))
    }
}

class FilterRuleTest {
    @Test fun `filters need a word because HQ refuses an empty search`() {
        org.junit.Assert.assertFalse(SearchMatch.canFilter(""))
        org.junit.Assert.assertFalse(SearchMatch.canFilter("   \n"))
        org.junit.Assert.assertTrue(SearchMatch.canFilter(" plan "))
    }
}
