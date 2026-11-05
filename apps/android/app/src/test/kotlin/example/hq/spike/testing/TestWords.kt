package example.hq.spike.testing

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import example.hq.spike.ui.shell.Words

/** The generated catalogue as the screens read it. Needs a Robolectric test. */
fun testWords(): Words = Words(ApplicationProvider.getApplicationContext<Context>().resources)
