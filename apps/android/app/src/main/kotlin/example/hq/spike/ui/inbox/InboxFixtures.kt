package example.hq.spike.ui.inbox

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
    val base = InboxUi(loaded = true, needs = needs, updates = updates, done = done, checkedAt = "14:32")
    return when (name) {
        "i-needs" -> base
        "i-updates" -> base.copy(tab = InboxTab.Updates)
        "i-clear" -> base.copy(tab = InboxTab.Updates, cleared = setOf("u3"), line = InboxLine("Update cleared", "Undo"))
        "i-details" -> base.copy(details = needs[1])
        "i-caught" -> base.copy(needs = emptyList(), updates = emptyList())
        "i-answering" -> base.copy(answering = mapOf("1" to Answer.Sending, "2" to Answer.Failed("Couldn't send your answer. HQ didn't reply. It may be restarting.")))
        else -> null
    }
}
