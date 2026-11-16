package example.hq.spike.ui.chat

import example.hq.spike.ui.chat.blocks.vaultPictureOf
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class VaultPictureTest {
    private val hash = "ab" + "cd".repeat(31)

    @Test
    fun a_link_relative_to_the_note_resolves_into_the_picture_store() {
        assertEquals(
            "_media/cas/ab/$hash.webp",
            vaultPictureOf("Notebooks/Inbox/Imports", "../../../_media/cas/ab/$hash.webp"),
        )
        assertEquals("_media/cas/ab/$hash.webp", vaultPictureOf("", "_media/cas/ab/$hash.webp"))
    }

    @Test
    fun anything_outside_the_store_or_outside_the_vault_stays_blocked() {
        assertNull(vaultPictureOf("Notebooks", "https://example.com/a.png"))
        assertNull(vaultPictureOf("Notebooks", "/etc/hosts"))
        assertNull(vaultPictureOf("Notebooks", "../../outside.png"))
        assertNull(vaultPictureOf("Notebooks", "pictures/a.png"))
        assertNull(vaultPictureOf("Notebooks", "..\\_media\\cas\\a.png"))
        assertNull(vaultPictureOf("Notebooks", "../_media/web/upload.png"))
    }
}
