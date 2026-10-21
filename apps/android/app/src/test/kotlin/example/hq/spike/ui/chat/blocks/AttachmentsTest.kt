package example.hq.spike.ui.chat.blocks

import org.junit.Assert.assertEquals
import org.junit.Test

class AttachmentsTest {
    private val marker = """ <hq-attachments>[{"name":"Battery-checklist.docx","path":"_media/web/d/x.docx","mime":"application/x","size":24576}]</hq-attachments>"""

    @Test fun theMarkerBecomesFilesAndLeavesTheText() {
        val (text, files) = Attachments.split("Here you go.$marker")
        assertEquals("Here you go.", text)
        val f = files.single()
        assertEquals("Word document", f.kind)
        assertEquals("24 KB", f.sizeWords)
        assertEquals("_media/web/d/x.docx", f.path)
    }

    @Test fun aMessageWithNoMarkerIsUnchanged() {
        assertEquals("plain" to emptyList<Attachment>(), Attachments.split("plain"))
    }

    @Test fun aMarkerCutOffInAPreviewIsStillStripped() {
        assertEquals("hi", Attachments.split("hi <hq-attachments>[{\"na").first)
    }

    @Test fun sizesReadInTheNearestUnit() {
        fun words(n: Long) = Attachment("a.bin", "p", "", n).sizeWords
        assertEquals("900 B", words(900))
        assertEquals("1.5 MB", words(1_572_864))
    }

    @Test fun aPictureIsNamedByItsMimeWhenTheNameHasNoExtension() {
        assertEquals("Picture", Attachment("photo", "p", "image/png", 1).kind)
        assertEquals("File", Attachment("blob", "p", "", 1).kind)
    }
}
