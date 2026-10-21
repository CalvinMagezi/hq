package example.hq.spike.ui.chat.blocks

import org.junit.Assert.assertEquals
import org.junit.Test

class NoteTextTest {
    @Test fun frontmatterIsDropped() {
        assertEquals("## Title\n\nBody", NoteText.withoutFrontmatter("---\ntitle: x\ntags: [a]\n---\n## Title\n\nBody"))
    }

    @Test fun aNoteWithoutFrontmatterIsUntouched() {
        assertEquals("## Title\n---\nmore", NoteText.withoutFrontmatter("## Title\n---\nmore"))
    }

    @Test fun anUnclosedBlockIsLeftAlone() {
        assertEquals("---\ntitle: x\nBody", NoteText.withoutFrontmatter("---\ntitle: x\nBody"))
    }

    @Test fun aSecondBlockAddedByATool_isDroppedToo() {
        val doubled = "---\ntitle: ''\n---\n\n---\ntitle: Real\ntags: [a]\n---\n\n# Real title\n\nBody"
        assertEquals("# Real title\n\nBody", NoteText.withoutFrontmatter(doubled))
    }

    @Test fun aRuleAfterTheBlockIsKeptWhenWhatFollowsIsNotAKey() {
        assertEquals("---\nSome words\n---\nmore", NoteText.withoutFrontmatter("---\ntitle: x\n---\n---\nSome words\n---\nmore"))
    }

    @Test fun theEditorKeepsASecondBlockBecauseTheServerKeepsOnlyTheFirstOnSave() {
        val doubled = "---\ntitle: ''\n---\n\n---\ntitle: Real\ntags: [a]\n---\n\n# Real title"
        assertEquals("---\ntitle: Real\ntags: [a]\n---\n\n# Real title", NoteText.withoutFirstBlock(doubled))
        assertEquals("text", NoteText.withoutFirstBlock("text"))
    }

    @Test fun aRuleSectionThatIsNotYamlIsNeverHidden() {
        assertEquals("---\nhttps://example.com/a\n---\nmore", NoteText.withoutFrontmatter("---\ntitle: x\n---\n---\nhttps://example.com/a\n---\nmore"))
        assertEquals("---\nA plain sentence.\nNote: kept\n---\nmore", NoteText.withoutFrontmatter("---\ntitle: x\n---\n---\nA plain sentence.\nNote: kept\n---\nmore"))
    }

    @Test fun aLongSecondBlockIsLeftAlone() {
        val long = (1..40).joinToString("\n") { "k$it: v" }
        assertEquals("---\n$long\n---\nbody", NoteText.withoutFrontmatter("---\ntitle: x\n---\n---\n$long\n---\nbody"))
    }
}
