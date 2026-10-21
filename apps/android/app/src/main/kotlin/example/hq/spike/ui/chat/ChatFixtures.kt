package example.hq.spike.ui.chat

import example.hq.spike.data.ToolStep
import example.hq.spike.ui.chat.blocks.Block
import example.hq.spike.ui.chat.blocks.ChartParse
import example.hq.spike.ui.chat.blocks.parseBlocks
import java.time.LocalDate

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
        else -> richFixture(name)
    }
}

private val LONG_RUN = "Notebooks/Projects/Agent-HQ"

private fun reply(id: String, text: String, tools: List<ToolStep> = emptyList()) = UiMessage(id, Speaker.Reply, text, tools)

private fun chat(title: String, question: String, vararg replies: UiMessage) =
    ChatUiState(threadId = "t-$title", title = title, messages = listOf(UiMessage("u", Speaker.User, question)) + replies)

private fun chartFence(json: String) = "```chart\n$json\n```"

private val BAR = chartFence("""{"type":"bar","title":"Tests passed per day","labels":["Mon","Tue","Wed","Thu","Fri"],"series":[{"name":"Passed","values":[18,24,21,30,27]}]}""")
private val LINE = chartFence("""{"type":"line","title":"Weekly signups","labels":["W1","W2","W3","W4","W5"],"series":[{"name":"Web","values":[12,18,15,22,25]},{"name":"Mobile","values":[5,9,14,16,21]}]}""")
private val LINE_PAGE = chartFence("""{"type":"line","title":"Weekly signups","xLabel":"Week","labels":["W1","W2","W3","W4","W5"],"series":[{"name":"Web","values":[12,18,15,22,25]},{"name":"Mobile","values":[5,9,14,16,21]}]}""")
private val DONUT = chartFence("""{"type":"donut","title":"Where the time went","yLabel":"min","labels":["Reading notes","Writing","Waiting","Other"],"series":[{"name":"Minutes","values":[16,12,7,3]}]}""")
private val BAD = chartFence("""{"type":"bar","labels":["A","B","C"],"series":[{"name":"Runs","values":[4,7]}]}""")

private val SMALL_TABLE = """
|Run|Result|Time|
|---|---|---:|
|Run 1|Passed|2 h 10 m|
|Run 2|Passed|2 h 04 m|
|Run 3|Failed|41 m|
""".trimIndent()

private val WIDE_TABLE = """
|Test|Result|Time|Battery|Notes|
|---|---|---:|---:|---|
|Cold start|Passed|1.2 s|0%|Within the limit|
|Reboot|Failed|n/a|n/a|Needs a reboot window|
|Doze, 8 hours|Passed|8 h|3%|Alerts arrived on time|
|Warm start|Passed|0.6 s|0%|Within the limit|
|Share capture|Passed|2.1 s|1%|Saved to the inbox|
|Widget add|Passed|1.4 s|0%|Shows on the home screen|
|Low battery|Passed|3 h|4%|Alerts kept coming|
|Airplane mode|Failed|n/a|n/a|Needs a retest|
""".trimIndent()

private val NOTE_BODY = """
---
title: Samsung battery steps
---
## Samsung battery steps

Do these four things on the Galaxy A32 before the soak test starts.

1. Open Settings, then Battery.
2. Find HQ in the list of apps.
3. Choose Unrestricted.
4. Turn off "Remove permissions if app is unused".

> Samsung may undo this after a system update. Check again after each one.
""".trimIndent()

private fun richFixture(name: String?): ChatUiState? {
    val tomorrow = LocalDate.now().plusDays(1).toString()
    return when (name) {
        "rich" -> chat(
            "Samsung battery steps", "Write up the Samsung battery steps and say what the roadmap wants",
            reply(
                "r", "I saved the steps as a new note. The [[Mobile-First-Roadmap]] says to test on the A32 first.",
                listOf(
                    ToolStep("s1", "vault_read", "{\"path\":\"$LONG_RUN/Mobile-First-Roadmap.md\"}", null, null),
                    ToolStep("s2", "vault_read", "{\"path\":\"$LONG_RUN/Android-On-Device-Assessment.md\"}", null, null),
                    ToolStep("s3", "vault_write_note", "{\"path\":\"$LONG_RUN/Samsung battery steps.md\"}", null, null),
                    ToolStep("s4", "task_create", "{\"title\":\"Check Samsung battery whitelist\",\"due_date\":\"$tomorrow\",\"priority\":\"high\"}", null, null),
                ),
            ),
        )
        "chips" -> chat(
            "Samsung battery steps", "Which notes did you use?",
            reply("r", "I read these.", listOf(
                ToolStep("s1", "vault_read", "{\"path\":\"$LONG_RUN/Roadmap.md\"}", null, null),
                ToolStep("s2", "vault_read", "{\"path\":\"$LONG_RUN/Opening-One.md\"}", null, null),
                ToolStep("s3", "vault_read", "{\"path\":\"$LONG_RUN/Old-plan.md\"}", null, null),
            )),
        ).let { it.copy(openingNote = "$LONG_RUN/Opening-One.md", missingNotes = setOf("$LONG_RUN/Old-plan.md")) }
        "note" -> chat("Samsung battery steps", "Write up the Samsung battery steps and say what the roadmap wants").copy(
            note = NoteSheet("$LONG_RUN/Samsung battery steps.md", "Samsung battery steps", "Projects › Agent-HQ", NOTE_BODY, truncated = false, savedAge = "just now"),
        )
        "tables" -> chat(
            "Soak test results", "Show me the soak test results",
            reply("r", "Three runs finished.\n\n$SMALL_TABLE\n\nThe latest run, test by test:\n\n$WIDE_TABLE"),
        )
        "tablepage" -> chat("Soak test results", "Show me the soak test results").copy(
            page = Page.Table(parseBlocks(WIDE_TABLE).filterIsInstance<Block.Table>().first(), "Soak test results · 8 rows · 5 columns"),
        )
        "bar" -> chat("Test results", "How many tests passed each day?", reply("r", "Passes climbed most days.\n\n$BAR"))
        "line" -> chat("Signups", "Show weekly signups", reply("r", "Both are growing.\n\n$LINE"))
        "chartpage" -> chat("Signups", "Show weekly signups").copy(page = Page.Chart(spec(LINE_PAGE)))
        "donutpage" -> chat("Time", "Where did the time go?").copy(page = Page.Chart(spec(DONUT)))
        "donut" -> chat(
            "Time", "Where did the time go?",
            reply("r", "Reading notes took 42% of the 38 minutes. Waiting took 18%.\n\n$DONUT\n\nTo check it yourself:\n\n```bash\nadb shell dumpsys deviceidle whitelist\nadb shell cmd appops set <app> RUN_ANY_IN_BACKGROUND allow\nadb shell settings get global app_standby_enabled\n```"),
        )
        "chartfail" -> chat("Test results", "Chart it", reply("r", "Here is the chart.\n\n$BAD"))
        "blocks" -> chat(
            "Battery", "What should I do?",
            reply("r", "Two things to do. The [Android battery guide](https://example.com/battery) has the details.\n\n> Samsung may undo this after an update.\n\n- [x] Allow background activity\n- [ ] Turn off automatic removal\n\n![The settings screen](https://example.com/shot.png)"),
        )
        "files" -> chat(
            "Battery steps", "Can you check this?",
            reply("r", "Here is the checklist I made.\n\n<hq-attachments>[{\"name\":\"Battery-checklist.docx\",\"path\":\"_media/web/d/a.docx\",\"mime\":\"application/x\",\"size\":24576}]</hq-attachments>"),
        ).let { it.copy(messages = it.messages.map { m -> UiMessage.saved(m.id, m.speaker, m.text) }) }
        "context" -> ChatUiState(contexts = listOf("$LONG_RUN/Mobile-First-Roadmap.md"))
        else -> null
    }
}

private fun spec(fence: String) = ((parseBlocks(fence).first() as Block.ChartBlock).result as ChartParse.Ok).spec
