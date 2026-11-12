package example.hq.spike.ui.chat.blocks

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ContactLinksTest {
    @Test fun readsTheOneShape() {
        assertEquals(ContactLink(42), ContactLinks.parse("hq://contacts/contact/42"))
    }

    @Test fun refusesAnythingElse() {
        listOf(
            "hq://contacts/contact/42?x=1",
            "hq://contacts/contact/42#frag",
            "hq://contacts/contact/-1",
            "hq://contacts/contact/99999999999999999999",
            "hq://contacts/contact/abc",
            "hq://contacts/contact/1/2",
            "hq://contacts/contact/",
            "hq://calendar/contact/1",
            "hq://contacts@evil.example/contact/1",
            "https://contacts/contact/1",
            "content://com.android.contacts/contacts/1",
        ).forEach { assertNull(it, ContactLinks.parse(it)) }
    }
}
