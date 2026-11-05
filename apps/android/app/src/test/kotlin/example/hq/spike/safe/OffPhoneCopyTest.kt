package example.hq.spike.safe

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class OffPhoneCopyTest {
    @Test fun onlyThePhonesOwnStorageCountsAsOnThePhone() {
        assertTrue(NotesCopy.onThisPhone("com.android.externalstorage.documents", "primary:Documents"))
        assertFalse(NotesCopy.onThisPhone("com.android.externalstorage.documents", "1A2B-3C4D:Backups"))
        assertTrue(NotesCopy.onThisPhone("com.android.providers.downloads.documents", "12"))
        assertTrue(NotesCopy.onThisPhone("com.android.providers.media.documents", "image:1"))
        assertFalse(NotesCopy.onThisPhone("com.google.android.apps.docs.storage", "abc"))
        assertFalse(NotesCopy.onThisPhone(null, null))
    }

    @Test fun aCopyOffThePhoneIsRecordedSeparately() {
        val dir = java.nio.file.Files.createTempDirectory("copy").toFile()
        try {
            NotesCopy.recordCopy(dir, 100L, onPhone = true)
            assertEquals(100L, NotesCopy.lastCopy(dir))
            assertNull(NotesCopy.lastCopyOffPhone(dir))
            NotesCopy.recordCopy(dir, 200L, onPhone = false)
            assertEquals(200L, NotesCopy.lastCopy(dir))
            assertEquals(200L, NotesCopy.lastCopyOffPhone(dir))
            NotesCopy.recordCopy(dir, 300L, onPhone = true)
            assertEquals(200L, NotesCopy.lastCopyOffPhone(dir))
        } finally { dir.deleteRecursively() }
    }
}
