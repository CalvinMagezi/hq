package example.hq.spike.testing

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import org.junit.Assert.assertTrue
import java.io.File

/**
 * Compares what the rule shows with `src/test/resources/screenshots/<name>.png` (resolutions.md D-11).
 * Run with `-Phq.recordScreenshots=true` to write the golden file after an intended change.
 */
object Screenshots {
    private val dir = File("src/test/resources/screenshots")
    /** Share of pixels that may differ: font hinting moves a few edge pixels between machines. */
    private const val MAX_DIFFERENT = 0.01
    private const val CHANNEL_TOLERANCE = 16

    /** Draws the activity's content straight to a bitmap; Robolectric's native graphics paint it. */
    private fun capture(rule: AndroidComposeTestRule<*, out ComponentActivity>): Bitmap {
        rule.waitForIdle()
        val view = rule.activity.window.decorView.findViewById<android.view.View>(android.R.id.content)
        val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bitmap))
        return bitmap
    }

    fun assertMatches(rule: AndroidComposeTestRule<*, out ComponentActivity>, name: String) {
        val shot = capture(rule)
        val golden = File(dir, "$name.png")
        if (System.getProperty("hq.recordScreenshots") == "true" || !golden.exists()) {
            dir.mkdirs()
            golden.outputStream().use { shot.compress(Bitmap.CompressFormat.PNG, 100, it) }
            assertTrue("Recorded $golden; commit it and run again", System.getProperty("hq.recordScreenshots") == "true")
            return
        }
        val want = BitmapFactory.decodeFile(golden.path)
        assertTrue("$name: size ${shot.width}x${shot.height}, golden ${want.width}x${want.height}", shot.width == want.width && shot.height == want.height)
        var different = 0
        for (y in 0 until shot.height) for (x in 0 until shot.width) {
            if (!close(shot.getPixel(x, y), want.getPixel(x, y))) different++
        }
        val share = different.toDouble() / (shot.width * shot.height)
        assertTrue("$name: ${"%.2f".format(share * 100)}% of pixels differ from $golden", share <= MAX_DIFFERENT)
    }

    private fun close(a: Int, b: Int): Boolean =
        listOf(0, 8, 16, 24).all { shift -> kotlin.math.abs((a shr shift and 0xFF) - (b shr shift and 0xFF)) <= CHANNEL_TOLERANCE }
}
