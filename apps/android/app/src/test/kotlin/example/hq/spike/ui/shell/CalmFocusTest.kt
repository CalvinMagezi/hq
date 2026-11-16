package example.hq.spike.ui.shell

import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Calm UI, acceptance 2: status surfaces (snackbars, banners, offers, problem bars) announce
 * politely without stealing focus.
 */
class CalmFocusTest {
    private val statusFiles = setOf(
        "Undo.kt",
        "Attention.kt",
        "ChatBanner.kt",
        "ProblemBar.kt",
        "OfferSlot.kt",
        "Offers.kt",
    )

    private fun uiDir(): File {
        var dir: File? = File(".").canonicalFile
        while (dir != null) {
            val direct = File(dir, "src/main/kotlin/example/hq/spike/ui")
            if (direct.isDirectory) return direct
            val fromRoot = File(dir, "apps/android/app/src/main/kotlin/example/hq/spike/ui")
            if (fromRoot.isDirectory) return fromRoot
            dir = dir.parentFile
        }
        error("uiDir() does not exist from ${File(".").canonicalPath}")
    }

    private fun stripComments(source: String): String =
        source
            .replace(Regex("/\\*[\\s\\S]*?\\*/"), "")
            .lines()
            .joinToString("\n") { it.replace(Regex("//.*$"), "") }

    private fun functionBody(source: String, name: String): String {
        val start = Regex("""\bfun\s+$name\s*\(""").find(source)?.range?.first
            ?: error("Could not find fun $name")
        val openBrace = source.indexOf('{', start)
        assertTrue("Could not find opening brace for fun $name", openBrace >= 0)
        var depth = 0
        for (i in openBrace until source.length) {
            when (source[i]) {
                '{' -> depth++
                '}' -> {
                    depth--
                    if (depth == 0) return source.substring(openBrace, i + 1)
                }
            }
        }
        error("Unclosed brace in fun $name")
    }

    @Test
    fun statusComposablesNeverRequestFocus() {
        val dir = uiDir()
        assertTrue("uiDir() must exist: ${dir.path}", dir.isDirectory)
        val ktFiles = dir.walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()
        assertTrue("Expected .kt files in ${dir.path}", ktFiles.isNotEmpty())

        val focusPattern = Regex("\\b(requestFocus|FocusRequester)\\b")
        val statusFunPattern = Regex("fun\\s+\\w*(Snackbar|Banner|Offer|ProblemBar|Notice)\\w*")
        val problems = buildList {
            ktFiles.forEach { file ->
                val text = stripComments(file.readText())
                if (!focusPattern.containsMatchIn(text)) return@forEach
                if (file.name in statusFiles || statusFunPattern.containsMatchIn(text)) {
                    add("${file.name} uses requestFocus/FocusRequester alongside status UI")
                }
            }
        }
        assertTrue(problems.joinToString("\n"), problems.isEmpty())
    }

    @Test
    fun snackbarRowUsesPoliteLiveRegion() {
        val undoSource = stripComments(File(uiDir(), "shell/Undo.kt").readText())
        val snackbarRowBody = functionBody(undoSource, "SnackbarRow")
        assertTrue(
            "SnackbarRow body must set liveRegion = LiveRegionMode.Polite",
            Regex("""liveRegion\s*=\s*LiveRegionMode\.Polite""").containsMatchIn(snackbarRowBody),
        )
    }
}
