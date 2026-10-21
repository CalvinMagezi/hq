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
    val base = TasksUi(loaded = true, tasks = tasks, today = today)
    return when (name) {
        "t-today" -> base
        "t-upcoming" -> base.copy(tab = TaskTab.Upcoming)
        "t-all" -> base.copy(tab = TaskTab.All)
        "t-quick" -> base.copy(quick = QuickState("Check Samsung battery whitelist tomorrow !high #inbox"))
        "t-more" -> base.copy(quick = QuickState("Check Samsung battery whitelist", more = true, priority = "high", due = today.plusDays(1)))
        "t-ready" -> base.copy(detail = TaskDetail(tasks[1], loading = false))
        "t-detail" -> base.copy(detail = TaskDetail(tasks[3].copy(description = "A native view for the whole app."), subtasks = listOf(t("a", "Map the current mobile UI", "complete"), t("b", "Draft the Compose design"), t("c", "Spike chat for T5")), loading = false))
        else -> null
    }
}
