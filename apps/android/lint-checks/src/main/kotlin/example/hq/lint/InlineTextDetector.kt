package example.hq.lint

import com.android.tools.lint.detector.api.Category
import com.android.tools.lint.detector.api.Detector
import com.android.tools.lint.detector.api.Implementation
import com.android.tools.lint.detector.api.Issue
import com.android.tools.lint.detector.api.JavaContext
import com.android.tools.lint.detector.api.Scope
import com.android.tools.lint.detector.api.Severity
import com.android.tools.lint.detector.api.SourceCodeScanner
import com.intellij.psi.PsiMethod
import org.jetbrains.uast.UCallExpression
import org.jetbrains.uast.UExpression
import org.jetbrains.uast.ULiteralExpression
import org.jetbrains.uast.UPolyadicExpression
import org.jetbrains.uast.evaluateString

/** A word a person reads written as a literal in a composable, instead of a catalogue string (N20). */
class InlineTextDetector : Detector(), SourceCodeScanner {
    override fun getApplicableMethodNames() = TEXT_CALLS

    override fun visitMethodCall(context: JavaContext, node: UCallExpression, method: PsiMethod) {
        val args = context.evaluator.computeArgumentMapping(node, method)
        for ((arg, param) in args) {
            if (param.name !in TEXT_PARAMS || !isWords(arg)) continue
            context.report(ISSUE, arg, context.getLocation(arg), MESSAGE)
        }
    }

    private fun isWords(arg: UExpression): Boolean {
        if (arg !is ULiteralExpression && arg !is UPolyadicExpression) return false
        val text = arg.evaluateString() ?: return false
        return LETTERS.containsMatchIn(text)
    }

    companion object {
        private val TEXT_CALLS = listOf("Text", "BasicText")
        private val TEXT_PARAMS = setOf("text", "contentDescription")
        private val LETTERS = Regex("[A-Za-z]{2,}")
        private const val MESSAGE =
            "Take these words from the string catalogue (apps/strings/en.yaml) with stringResource."

        val ISSUE = Issue.create(
            id = "HqInlineText",
            briefDescription = "Words written inline in a composable",
            explanation = "Every sentence the app shows comes from one catalogue of whole sentences " +
                "(apps/strings/en.yaml), so the phone and the web say the same thing and it can be translated.",
            category = Category.I18N,
            priority = 6,
            severity = Severity.ERROR,
            implementation = Implementation(InlineTextDetector::class.java, Scope.JAVA_FILE_SCOPE),
        )
    }
}
