package example.hq.spike.ui.helpers

import example.hq.spike.data.HqClient.HelperHost
import example.hq.spike.data.HqClient.HelperRow
import java.time.LocalDateTime
import java.time.ZoneOffset

/** Fixed Helpers screens for checking against the design with no hq: `--es fixture h-list`, `h-ok`, `h-page`, `h-start`, `h-tech`, `h-stop`, `h-real`. */
fun helpersFixture(name: String?): HelpersUi? {
    val now = LocalDateTime.now(ZoneOffset.UTC)
    fun at(minutes: Long) = now.minusMinutes(minutes).toString().replace('T', ' ').substringBefore('.')
    fun row(id: String, harness: String, label: String, status: String, agent: String, minutes: Long, seen: Long = 1, host: String = "Mac mini", reachable: Boolean = true, alive: Boolean? = true, detail: String? = null, goal: String? = null) =
        HelperRow(id, harness, label, host, "/work/$id", status, agent, at(seen), at(minutes), goal, null, alive, reachable, detail, false)
    val rows = listOf(
        row("a", "claude-code", "Move the build from scripts to Gradle", "running", "blocked", 40),
        row("b", "codex", "Build the Android app", "running", "working", 12),
        row("c", "claude-code", "Fix the backup script", "exited", "done", 190, seen = 120),
        row("d", "codex", "Check the tunnel notes", "exited", "done", 400, seen = 300),
        row("e", "claude-code", "", "stopped", "idle", 600, seen = 500, goal = "Run pwd in the new session and report its exact output"),
    )
    val ask = BlockedPrompt(AskKind.Edit, "Chat.dc.html", "Do you want to make this edit to Chat.dc.html?")
    val screen = listOf(
        "╭────────────────────────────────────────╮",
        "│ Edit file                              │",
        "│ Chat.dc.html                           │",
        "╰────────────────────────────────────────╯",
        "Do you want to make this edit to Chat.dc.html?",
        "❯ 1. Yes",
        "  2. Yes, allow all edits during this session",
        "  3. No, and tell Claude what to do differently",
    )
    val base = HelpersUi(loaded = true, rows = rows, asks = mapOf("a" to ask), lastWords = mapOf("b" to "Updating the build files"), now = now)
    return when (name) {
        "h-list" -> base
        "h-needs" -> base.copy(filter = HelperFilter.Needs)
        "h-finished" -> base.copy(filter = HelperFilter.Finished)
        "h-empty" -> HelpersUi(loaded = true, now = now)
        "h-ok" -> base.copy(open = HelperOpen("a", HelperPage.Ask, screen = screen))
        "h-raw" -> base.copy(open = HelperOpen("a", HelperPage.Ask, screen = screen, raw = true))
        "h-page" -> base.copy(open = HelperOpen("b", HelperPage.Detail, screen = listOf("● Updating the build files", "  ⎿ Read build.gradle.kts")))
        "h-tech" -> base.copy(open = HelperOpen("b", HelperPage.Detail, technical = true, screen = screen))
        "h-stop" -> base.copy(open = HelperOpen("b", HelperPage.Detail, confirmStop = true))
        "h-menu" -> base.copy(open = HelperOpen("b", HelperPage.Detail, menu = true))
        "h-start" -> base.copy(start = StartForm("Move the build from scripts to Gradle", hosts = listOf(HelperHost("Mac mini", true, null)), host = "Mac mini"))
        "h-nohost" -> base.copy(start = StartForm("Move the build", hosts = listOf(HelperHost("laptop", false, "no gate key"))))
        "h-real" -> realHelpers(now)
        "h-real-open" -> realHelpers(now).copy(finishedOpen = true)
        "h-real-page" -> realHelpers(now).let { it.copy(open = HelperOpen("r0", HelperPage.Detail)) }
        else -> null
    }
}

/** The shapes the live list has on the phone: one running helper whose computer cannot be reached, and many stopped ones. */
private fun realHelpers(now: LocalDateTime): HelpersUi {
    fun at(hours: Long) = now.minusHours(hours).toString().replace('T', ' ').substringBefore('.')
    val rows = listOf(
        HelperRow("r0", "claude-code", "", "laptop", "/Users/someone/work/egress-probe", "running", "unknown", at(30), at(30), null, null, null, false, "host 'laptop' unreachable: no gate key at /data/user/0/example.hq.spike/files/hqhome/.hq/gate/laptop.key; make one with `hq host gate-key <host>`", false),
        HelperRow("r1", "claude-code", "egress-probe", "laptop", "/work/p", "stopped", "blocked", at(30), at(31), null, null, false, true, null, false),
        HelperRow("r2", "claude-code", "phase2-mcp-check-2", "laptop", "/work/p", "stopped", "done", at(31), at(31), "Prove a host session reaches HQ over the new tunnel URL for MCP", null, false, true, null, false),
        HelperRow("r3", "claude-code", "pwd-check", "laptop", "/tmp", "stopped", "idle", at(250), at(250), "Run pwd in the new Claude Code session and report its exact output.", null, false, true, null, false),
        HelperRow("r4", "claude-code", "", "laptop", "/Users/someone/hq-orch-probe", "stopped", "done", at(260), at(260), "print ready then done", null, false, true, null, false),
        HelperRow("r5", "cursor", "", "laptop", "/Users/someone/Documents/HQ/cursor-probe 2026-10-08 1102", "stopped", "idle", at(310), at(310), null, null, false, true, null, false),
        HelperRow("r6", "cursor", "", "laptop", "/Users/someone/Documents/HQ/cursor-probe 2026-10-08 1215", "stopped", "idle", at(309), at(309), null, null, false, true, null, false),
        HelperRow("r7", "claude-kola", "", "laptop", "/Users/someone/Documents/HQ/kola-check", "stopped", "idle", at(308), at(308), null, null, false, true, null, false),
        HelperRow("r8", "pi", "", "laptop", "/Users/someone/Documents/HQ/pi-check", "stopped", "idle", at(307), at(307), null, null, false, true, null, false),
    )
    return HelpersUi(loaded = true, rows = rows, now = now)
}
