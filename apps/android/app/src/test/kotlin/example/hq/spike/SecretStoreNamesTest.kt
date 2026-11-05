package example.hq.spike

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** Check 3 of wave-0/03-data-transfer.md: a key copied from another phone cannot open here, so it is not listed. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class SecretStoreNamesTest {
    @Test fun aSealedValueThatDoesNotOpenIsNotListed() {
        val context: Context = ApplicationProvider.getApplicationContext()
        context.getSharedPreferences("secrets", Context.MODE_PRIVATE).edit()
            .clear()
            .putString("OPENROUTER_API_KEY", "AAAAAAAAAAAAAAAA:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
            .putString("TELEGRAM_BOT_TOKEN", "not sealed at all")
            .commit()
        assertEquals(emptyList<String>(), SecretStore.names(context))
    }
}
