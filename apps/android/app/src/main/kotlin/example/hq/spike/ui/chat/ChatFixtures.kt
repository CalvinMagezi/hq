package example.hq.spike.ui.chat

import example.hq.spike.data.ToolStep

/**
 * Fixed screens for checking the layout against the design on a phone with no hq running:
 * `adb shell am start -n example.hq.spike/.ShellActivity --es fixture streaming`.
 */
fun chatFixture(name: String?): ChatUiState? {
    val checked = ToolStep("c1", "task_list", null, null, null)
    val read = ToolStep("c2", "vault_read", "{\"path\":\"Notebooks/Projects/Android-On-Device-Assessment.md\"}", null, null)
    val settled = listOf(
        UiMessage("u1", Speaker.User, "How do I stop Samsung putting the app to sleep?"),
        UiMessage("r1", Speaker.Reply, "Open Settings, then Battery, find AgentHQ and choose Unrestricted. Also turn off \"Remove permissions if app is unused\".", listOf(checked, read)),
        UiMessage("u2", Speaker.User, "Write the steps down for me."),
    )
    val rows = listOf(
        ChatRow("t1", "Samsung battery steps", "3m ago · Nothing is blocking it", Group.Today, running = false, unread = false),
        ChatRow("t2", "What is blocking the soak test?", "", Group.Today, running = true, unread = false),
        ChatRow("t3", "Telegram check", "The relay is connected", Group.Yesterday, running = false, unread = true),
        ChatRow("t4", "Backup prune", "It needs a yes from you", Group.Week, running = false, unread = false),
    )
    val open = ChatUiState(threadId = "t1", title = "Samsung battery steps", messages = settled)
    return when (name) {
        "empty" -> ChatUiState()
        "streaming" -> open.copy(
            live = LiveReply(steps = listOf(LiveStep(checked, true), LiveStep(read, true), LiveStep(ToolStep("c3", "vault_write_note", null, null, null), false))),
        )
        "chats" -> open.copy(messages = settled.take(2), chats = rows, chatsOpen = true, notice = Notice("Chat archived", "Undo"))
        "offline" -> open.copy(messages = settled.take(2), connected = false, draft = "Check the backup status")
        "failed" -> open.copy(failed = true)
        else -> null
    }
}
