package example.hq.spike.ui.inbox

import example.hq.spike.data.HqClient.InboxItem
import java.time.OffsetDateTime
import java.time.ZoneOffset

/** Fixed Inbox screens for checking against the design with no hq: `--es fixture i-needs`, `i-updates`, `i-details`, `i-caught`. */
fun inboxFixture(name: String?): InboxUi? {
    fun ago(minutes: Long) = OffsetDateTime.now(ZoneOffset.UTC).minusMinutes(minutes).toString()
    fun card(id: String, kind: CardKind, title: String, body: String, where: String, minutes: Long, icon: CardIcon, canAllow: Boolean, label: String, sessionId: String? = null) =
        InboxCard(id, kind, title, body, where, ago(minutes), icon, sessionId, canAllow, label, null, where, "pending")
    val needs = listOf(
        card("1", CardKind.HelperAsks, "Claude needs your OK", "It stopped to ask: Do you want to edit Chat.dc.html?", "Mac mini", 2, CardIcon.Sparkle, true, "Allow once", "s1"),
        card("2", CardKind.Approval, "Delete 3 old backups?", "The nightly backup wants to remove the 3 oldest copies, from before Sep 21.", "Backup task", 5, CardIcon.Info, true, "Allow"),
        card("3", CardKind.HelperStuck, "A helper is stuck", "Codex keeps hitting the same question, so HQ stopped answering it.", "Mac mini", 60, CardIcon.Pause, false, "Take a look"),
    )
    val updates = listOf(
        card("u1", CardKind.Update, "Backup finished", "2,860 notes saved to your Mac.", "HQ", 20, CardIcon.Check, false, ""),
        card("u2", CardKind.Update, "A task is ready to check", "Nightly off-device backup. Tap to take a look.", "HQ", 60, CardIcon.Task, false, ""),
        card("u3", CardKind.Update, "Disk space is getting low", "82% used. The build folder is 41 GB.", "HQ", 120, CardIcon.Warning, false, ""),
        card("u4", CardKind.Update, "Telegram is back", "It was offline for 40 seconds.", "HQ", 180, CardIcon.Info, false, ""),
    )
    val done = listOf(
        DoneEntry("d1", "Allowed: Claude edits Chat.dc.html", ago(30)),
        DoneEntry("d2", "Not now: Delete 3 old backups", ago(45)),
    )
    val real = realInbox(::ago)
    val base = InboxUi(loaded = true, needs = needs, updates = updates, done = done, checkedAt = "14:32")
    return when (name) {
        "i-helpers" -> InboxUi(loaded = true, needs = helperNotices().needs, checkedAt = "14:32")
        "i-real" -> InboxUi(loaded = true, needs = real.needs, updates = real.updates, done = real.done, checkedAt = "14:32")
        "i-real-updates" -> InboxUi(loaded = true, tab = InboxTab.Updates, needs = real.needs, updates = real.updates, done = real.done, checkedAt = "14:32")
        "i-real-done" -> InboxUi(loaded = true, tab = InboxTab.Done, needs = real.needs, updates = real.updates, done = real.done, checkedAt = "14:32")
        "i-needs" -> base
        "i-updates" -> base.copy(tab = InboxTab.Updates)
        "i-clear" -> base.copy(tab = InboxTab.Updates, cleared = setOf("u3"), line = InboxLine("Update cleared", "Undo"))
        "i-details" -> base.copy(details = needs[1])
        "i-caught" -> base.copy(needs = emptyList(), updates = emptyList())
        "i-long" -> InboxUi(loaded = true, needs = listOf(longApproval(::ago)), checkedAt = "14:32")
        "i-long-details" -> longApproval(::ago).let { InboxUi(loaded = true, needs = listOf(it), details = it, checkedAt = "14:32") }
        "i-offline" -> InboxUi(loaded = true, offline = true)
        "i-answering" -> base.copy(answering = mapOf("1" to Answer.Sending, "2" to Answer.Failed("Couldn't send your answer. HQ didn't reply. It may be restarting.")))
        else -> null
    }
}

/** The shapes the live server sends: task reviews with a whole description, notices with backticks, answers from several days. */
/** An approval too long for a card, made the way the app makes every card, so the card and its page show what a real one would. */
private fun longApproval(ago: (Long) -> String): InboxCard {
    val body = "The export clean-up wants to remove every export older than 30 days from the exports folder on this phone. " +
        "That is 214 files, about 1.2 GB. It keeps the newest copy of each note and anything you shared in the last week. " +
        "Nothing is removed from your notes themselves, only the exported copies. It cannot be undone, but each note can be exported again. " +
        "It will run once now and then every Sunday night unless you turn it off in Settings."
    return InboxWords.card(InboxItem("val_9", "action_needed", "Run the export clean-up?", body, "pending", ago(7), null, "export-task", null, listOf("approve", "reject")))
}

private fun realInbox(ago: (Long) -> String): InboxLists {
    val long = "Add a service to the app so it can be chosen under Settings, Apps, Choose default apps. On the test phone check that the side button reaches the app; that a microphone-type foreground service can be started with no visible screen (the official while-in-use exemption says it should); and what happens after a reboot and unlock. Exit: a written result for each case, with the evidence kept in the project folder for the next review."
    fun item(id: String, title: String, body: String, minutes: Long, source: String?, actions: List<String>, state: String = "pending", kind: String = "action_needed") =
        InboxItem(id, kind, title, body, state, ago(minutes), null, source, null, actions)
    val review = listOf("approve", "reject")
    val pending = listOf(
        item("r1", "FR-58: Voice entry: assistant service skeleton and the first on-device test round", long, 480, "task_ready_for_review", review),
        item("r2", "FR-61: Native chat screen against the existing REST and web-socket API", "Spike a native chat screen that talks to the local hq over the existing API, with streaming replies and the notifications list. Compare feel and effort with the WebView; decide whether the app moves to Gradle.", 120, "task_ready_for_review", review),
        item("r3", "FR-62: Userspace tunnel inside the app plus QR pairing", "Prototype a tunnel inside the app with no VPN permission, configured by scanning a QR code from `hq net`.", 118, "task_ready_for_review", review),
        item("u1", "Install check", "Native build installed on this phone.", 90, "backup", emptyList(), kind = "fyi"),
        item("u2", "Skills improved: note-authoring", "note-authoring (hints learned): These phrases should help select the workflow directly for similar requests.\nUndo any change with `hq skills revert <name>`.", 300, "skills", emptyList(), kind = "fyi"),
        item("u3", "Backup finished", "2,860 notes saved to your Mac.", 60 * 30, "backup", emptyList(), kind = "fyi"),
    )
    val history = listOf(
        item("h1", "FR-278: Phase 5: game layer", "", 40, "task_ready_for_review", emptyList(), state = "dismissed"),
        item("h2", "FR-277: Phase 4: employers and sponsors", "", 45, "task_ready_for_review", emptyList(), state = "dismissed"),
        item("h3", "Delete 3 old backups?", "", 60 * 26, "backup", emptyList(), state = "approved"),
        item("h4", "FR-289: Phone sign-in", "", 60 * 27, "task_ready_for_review", emptyList(), state = "dismissed"),
        item("h5", "FR-288: Passport and sharing", "", 60 * 24 * 4, "task_ready_for_review", emptyList(), state = "dismissed"),
    )
    return inboxLists(pending, history)
}

/** Two helpers at a Bash prompt as the supervisor words them: a short command the card can show whole, and one that wraps over five rows. */
private fun helperNotices(): InboxLists {
    fun notice(id: String, session: String, lines: List<String>) = InboxItem(
        id, "action_needed", "Harness session 'claude in hq-oss' is waiting for you",
        "Session $session on mac-mini is blocked on a prompt or approval:\n----- session output -----\n${lines.joinToString("\n")}\n----- end session output -----\nRead it with harness_session_logs.",
        "pending", OffsetDateTime.now(ZoneOffset.UTC).minusMinutes(3).toString(), null, "session-supervisor", null, listOf("approve", "reject"),
    )
    return inboxLists(listOf(notice("p1", "c2", example.hq.spike.ui.helpers.SHORT_COMMAND), notice("p2", "c1", example.hq.spike.ui.helpers.LONG_COMMAND)), emptyList())
}
