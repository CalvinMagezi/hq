package example.hq.lint

import com.android.tools.lint.client.api.IssueRegistry
import com.android.tools.lint.client.api.Vendor
import com.android.tools.lint.detector.api.CURRENT_API

class HqIssueRegistry : IssueRegistry() {
    override val issues = listOf(InlineTextDetector.ISSUE)
    override val api = CURRENT_API
    override val vendor = Vendor(vendorName = "Agent HQ", identifier = "agenthq")
}
