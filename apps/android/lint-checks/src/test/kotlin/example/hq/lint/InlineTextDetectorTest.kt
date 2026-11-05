package example.hq.lint

import com.android.tools.lint.checks.infrastructure.LintDetectorTest
import com.android.tools.lint.detector.api.Detector
import com.android.tools.lint.detector.api.Issue

class InlineTextDetectorTest : LintDetectorTest() {
    override fun getDetector(): Detector = InlineTextDetector()
    override fun getIssues(): List<Issue> = listOf(InlineTextDetector.ISSUE)

    private val textStub = kotlin(
        """
        package androidx.compose.material3
        fun Text(text: String, contentDescription: String? = null) {}
        """,
    ).indented()

    fun testInlineWordsAreReported() {
        lint().files(
            textStub,
            kotlin(
                """
                package example.hq
                import androidx.compose.material3.Text
                fun Screen() { Text("Starting HQ") }
                """,
            ).indented(),
        ).allowMissingSdk().run().expectErrorCount(1)
    }

    fun testCatalogueStringsAndSymbolsAreNot() {
        lint().files(
            textStub,
            kotlin(
                """
                package example.hq
                import androidx.compose.material3.Text
                fun Screen(words: String) { Text(words); Text("·"); Text("12") }
                """,
            ).indented(),
        ).allowMissingSdk().run().expectClean()
    }
}
