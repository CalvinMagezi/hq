package example.hq.spike.ui.tasks

import example.hq.spike.data.HqClient.TaskItem
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneOffset

/** Fixed Tasks screens for checking against the design with no hq: `--es fixture t-today`, `t-upcoming`, `t-all`, `t-quick`, `t-ready`. */
fun tasksFixture(name: String?): TasksUi? {
    val today = LocalDate.now()
    val hoursAgo = LocalDateTime.now(ZoneOffset.UTC).minusHours(3).toString().replace('T', ' ').substringBefore('.')
    fun t(id: String, title: String, status: String = "to_do", priority: String? = null, due: LocalDate? = null, start: LocalDate? = null, reason: String? = null, steps: Pair<Int, Int> = 0 to 0, ready: String? = null, desc: String = "") =
        TaskItem(id, "T-$id", title, desc, status, priority, due?.toString(), start?.toString(), emptyList(), emptyList(), null, steps.second, steps.first, reason, ready, "2026-10-0${id.takeLast(1).ifEmpty { "1" }} 10:00:00")
    val tasks = listOf(
        t("1", "Soak, reboot test, cleanup of old copies", "blocked", reason = "needs a reboot window"),
        t("2", "Nightly off-device backup", "ready_for_review", ready = hoursAgo, desc = "Set up a nightly encrypted copy of the vault to the Mac. First run wrote one archive and verified 2,860 notes."),
        t("3", "Workstream A: first run for a phone-only user", "in_progress", steps = 1 to 3),
        t("4", "Workstream C: a native view instead of the WebView", "in_progress", priority = "high", due = today.plusDays(3)),
        t("5", "Check Samsung battery whitelist", priority = "high", start = today.plusDays(1)),
        t("6", "Release build and certificate renewal", due = today.plusDays(2)),
        t("7", "Shrink or retire the hub droplet", due = today.plusDays(2)),
        t("8", "Plan the winter soak test", due = today.plusDays(30)),
        t("9", "Tidy the vault folders"),
    )
    val stuckSince = LocalDateTime.now(ZoneOffset.UTC).minusDays(2).toString().replace('T', ' ').substringBefore('.')
    val withDetail = tasks.map { if (it.id == "1") it.copy(blockedSince = stuckSince, waitingOn = "you", priority = null) else it }
    val finished = listOf(t("d1", "Plan the cutover week", "complete").copy(completedAt = hoursAgo), t("d2", "Pick the soak test phone", "complete").copy(completedAt = hoursAgo))
    val put = listOf(t("x1", "Old idea: a desktop widget", "to_do").copy(archivedAt = hoursAgo))
    val helpers = listOf(example.hq.spike.data.HqClient.WorkSession("4", "hq", "claude-code", "/work/hq-oss"))
    val base = TasksUi(loaded = true, tasks = withDetail + finished, today = today, archived = put, sessions = helpers)
    val real = TasksUi(loaded = true, tasks = realTasks(today), today = today)
    return when (name) {
        "t-real" -> real
        "t-real-open" -> real.copy(expanded = setOf("waiting"))
        "t-real-all" -> real.copy(tab = TaskTab.All)
        "t-today" -> base
        "t-upcoming" -> base.copy(tab = TaskTab.Upcoming)
        "t-all" -> base.copy(tab = TaskTab.All)
        "t-select" -> base.copy(tab = TaskTab.All, selected = setOf("3", "4"))
        "t-select-status" -> base.copy(tab = TaskTab.All, selected = setOf("3", "4"), bulkPrompt = "status")
        "t-all-stuck" -> base.copy(tab = TaskTab.All, chip = AllChip.Stuck)
        "t-all-done" -> base.copy(tab = TaskTab.All, chip = AllChip.Done)
        "t-all-archived" -> base.copy(tab = TaskTab.All, chip = AllChip.Archived)
        "t-archived" -> base.copy(tab = TaskTab.All, chip = AllChip.Archived, detail = TaskDetail(put[0].copy(description = "Not needed now."), loading = false))
        "t-quick" -> base.copy(quick = QuickState("Check Samsung battery whitelist tomorrow !high #inbox"))
        "t-more" -> base.copy(quick = QuickState("Check Samsung battery whitelist", more = true, priority = "high", due = today.plusDays(1)))
        "t-ready" -> base.copy(detail = TaskDetail(tasks[1], loading = false))
        "t-detail" -> base.copy(detail = TaskDetail(tasks[3].copy(description = "A native view for the whole app."), subtasks = listOf(t("a", "Map the current mobile UI", "complete"), t("b", "Draft the Compose design"), t("c", "Spike chat for T5")), loading = false))
        "t-links" -> base.copy(detail = TaskDetail(tasks[3].copy(description = "STATE: deployed and verified at https://dev.example.com/app (signed in as admin). Docs: https://example.com/docs/setup.\nSession: https://example.com/code/session_01AB, then check (https://example.com/a)."), subtasks = emptyList(), loading = false))
        else -> null
    }
}

/** The shapes the live list has: dozens of tasks waiting to be checked with long titles, and to-dos that only start today. */
private fun realTasks(today: LocalDate): List<TaskItem> {
    val days = listOf(9L, 8L, 6L, 5L, 3L, 3L, 2L, 2L, 1L, 1L)
    val subjects = listOf(
        "Security: public user lookups return full user records (emails and phones) with no sign-in",
        "Production API: rollout stuck since last week (migrations init container crash-loops, 512 restarts); prod still serves the old image",
        "Phase 0: switch CI on for main (allow-list safe actions, pinned toolchain, required checks)",
        "Quick win: UTF-8 safe truncation (worker panic on multibyte instruction) and audit the route extractor",
        "Contracts import check lets a department-scoped admin import the master list",
        "Decide and apply the approval stage for imported contracts (all 93 are in draft)",
    )
    val ready = (1..74).map { i ->
        val ago = LocalDateTime.now(ZoneOffset.UTC).minusDays(days[i % days.size]).toString().replace('T', ' ').substringBefore('.')
        TaskItem("r$i", "FR-$i", subjects[i % subjects.size] + if (i > subjects.size) " (part $i)" else "", "", "ready_for_review", null, null, null, emptyList(), emptyList(), null, 0, 0, null, ago, "2026-10-1${i % 10} 10:00:00")
    }
    fun open(id: String, title: String, status: String, priority: String? = null, start: LocalDate? = null, due: LocalDate? = null, steps: Pair<Int, Int> = 0 to 0) =
        TaskItem(id, "FR-$id", title, "", status, priority, due?.toString(), start?.toString(), emptyList(), emptyList(), null, steps.second, steps.first, null, null, "2026-10-0${id.takeLast(1)} 10:00:00")
    return ready + listOf(
        open("901", "Phase 0: reconcile the two branches, port the access rules, audit, add the network policy and the image scan gate into main", "in_progress", "urgent"),
        open("902", "Harden the script runner: environment allowlist, process-group kill, resource limits, separate user", "in_progress", "high"),
        open("903", "Move the app to Gradle with Kotlin and Compose enabled, no behaviour change", "to_do", "high", start = today),
        open("904", "Push-to-talk loop latency through the chat API", "to_do", "high", start = today.minusDays(1)),
        open("905", "Cut over from the old cloud host to self-hosted infrastructure", "to_do", "urgent", due = today.minusDays(7), steps = 2 to 12),
        open("906", "Compliance pack for the first paying customer (tax, folio, register, privacy)", "to_do", "high", start = today.minusDays(20), due = today.plusDays(46), steps = 4 to 11),
    )
}
