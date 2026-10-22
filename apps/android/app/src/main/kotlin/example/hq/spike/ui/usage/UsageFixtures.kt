package example.hq.spike.ui.usage

/** A fixed top-bar chip for checking against the design with no hq: `--es balance copilot`, `openrouter`, `budget`, `exceeded` or `none`. */
fun balanceFixture(name: String?): BalanceChip? = when (name) {
    "copilot" -> BalanceChip("Copilot 912 left", "GitHub Copilot credits: 912 of 1.5k left, 8/h over 6h", ChipTarget.AiService)
    "openrouter" -> BalanceChip("OpenRouter \$4.20 left", "OpenRouter: \$4.20 left", ChipTarget.AiService)
    "today" -> BalanceChip("DeepSeek \$0.42 today", "DeepSeek: \$0.42 today", ChipTarget.AiService)
    "budget" -> BalanceChip("all 85%", "Budget all: \$4.25 of \$5.00 daily", ChipTarget.Spending, ChipTone.Warning)
    "exceeded" -> BalanceChip("all 120%", "Budget all: \$6.00 of \$5.00 daily", ChipTarget.Spending, ChipTone.Exceeded)
    else -> null
}
