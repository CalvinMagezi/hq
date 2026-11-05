package example.hq.spike

import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Test

/** No string literal in the app's source says what the connection rules retired; the words come from the catalogue. */
class RetiredPhrasesTest {
    private val retired = listOf("HQ is not answering", "HQ didn't reply", "HQ is not running", "HQ did not answer", "Not answering", "exit code")

    /** The string literals of a source file, with comments left out. */
    private fun literals(text: String): List<String> {
        val noComments = text.replace(Regex("/\\*[\\s\\S]*?\\*/"), "").lines().joinToString("\n") { it.replace(Regex("(^|\\s)//.*$"), "") }
        return Regex("\"((?:[^\"\\\\\\n]|\\\\.)*)\"").findAll(noComments).map { it.groupValues[1] }.toList()
    }

    @Test fun noSourceLiteralUsesARetiredPhrase() {
        val found = File("src/main").walkTopDown().filter { it.isFile && (it.extension == "kt" || it.extension == "java") }.flatMap { f ->
            literals(f.readText()).filter { lit -> retired.any { lit.contains(it, ignoreCase = it == "exit code") } }.map { "${f.name}: $it" }
        }.toList()
        assertTrue(found.joinToString("\n"), found.isEmpty())
    }
}
