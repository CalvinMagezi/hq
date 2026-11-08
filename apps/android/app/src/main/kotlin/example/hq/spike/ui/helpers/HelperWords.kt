package example.hq.spike.ui.helpers

import example.hq.spike.R
import example.hq.spike.data.HqClient
import example.hq.spike.data.HqClient.HelperRow
import example.hq.spike.ui.shell.Words
import java.time.Duration
import java.time.LocalDateTime
import java.time.format.DateTimeParseException

enum class HelperState { NeedsOk, Working, Ready, Offline, Lost, Finished, Stopped }

enum class AskKind { Edit, Create, Command, Plan, Other }

/** What a blocked helper is asking, read from the end of its terminal. */
data class BlockedPrompt(
    val kind: AskKind,
    /** The file as the dialog wrote it, a path or a name; null for anything that is not about one file. */
    val subject: String?,
    val question: String,
    /** The dialog as drawn: the lines leading up to the question, the question, the options and the cursor. Two prompts are the same only if all of it matches. */
    val block: List<String> = emptyList(),
    /** What it wants to do, in its own words: every line between the top of the dialog and the question. */
    val detail: List<String> = emptyList(),
    /** How many lines at the end of [block] are the question and what follows it: a later read must match at least these. */
    val core: Int = 0,
)

/** The verdict on a tap of Allow once or Not now, from a fresh read of the screen. */
enum class AnswerCheck { Ok, NoPrompt, Changed, BroadYes, NoCursor, NotLive }

/** A helper as a card: the job as the title, who and where, and one line on what it does or asks. */
data class HelperCard(
    val id: String,
    val title: String,
    val agent: String,
    val where: String,
    val state: HelperState,
    val line: String,
    /** "12 min" for work in progress, "2 h ago" for finished ones. */
    val time: String,
    val ask: BlockedPrompt?,
    val sessionId: String,
    /** True when the title is only "Claude helper": the row had no label, goal or task to name its job. */
    val generic: Boolean = false,
    /** The folder it ran in, to tell two nameless helpers apart. */
    val folder: String = "",
) {
    /** HQ's own work on this phone, which is told apart from a coding helper. */
    val own: Boolean get() = id.startsWith(HelperWords.EDGE_PREFIX)
}

/** The words the Helpers board uses, kept out of the screens so they can be tested. */
object HelperWords {
    private val BOX = Regex("[\\u2500-\\u257F\\u2580-\\u259F\\u25A0-\\u25FF\\u2700-\\u27BF\\u23BF\\u23FA\\u2733\\u273B]")
    private val QUESTION = Regex("\\?\\s*$")
    private val EDIT = Regex("(?i)\\b(?:edit|modify|change|update)\\s+(?:to\\s+)?(?:the\\s+|this\\s+)?(?:file\\s+)?[`'\"]?([\\w./~-]+\\.\\w+)")
    private val CREATE = Regex("(?i)\\b(?:create|write|make)\\s+(?:the\\s+|a\\s+|new\\s+)?(?:file\\s+)?[`'\"]?([\\w./~-]+\\.\\w+)")
    private val COMMAND = Regex("(?i)\\b(?:run|execute)\\b|\\bbash\\b|\\bproceed\\b|\\bcommand\\b")
    private const val MAX_TITLE = 80
    const val MAX_LINE = 140

    fun agentName(w: Words, harness: String): String = when (harness) {
        "claude-code", "claude" -> "Claude"
        "codex" -> "Codex"
        "cursor" -> "Cursor"
        "opencode" -> "OpenCode"
        "copilot", "copilot-cli" -> "Copilot"
        "hermes" -> "Hermes"
        "openclaw" -> "OpenClaw"
        else -> harness.split('-', '_').filter { it.isNotBlank() }.joinToString(" ") { it.replaceFirstChar(Char::uppercase) }.ifBlank { w.s(R.string.helpers_a_helper) }
    }

    fun placeName(w: Words, host: String): String = when (host) {
        "", "local", "native" -> w.s(R.string.helpers_this_phone)
        else -> w.s(R.string.helpers_your_host, host)
    }

    /** The job, never a folder name: the label, else the goal, else the task it serves, else a plain name. */
    fun isGeneric(row: HelperRow): Boolean = listOf(row.label, row.goal, row.taskTitle).none { !it.isNullOrBlank() }

    /** The last part of the folder a helper ran in, without a trailing slash. */
    fun folderOf(cwd: String): String = cwd.trimEnd('/').substringAfterLast('/').trim()

    /** Why a computer cannot be reached, in the owner's words. The server's own text names a missing key file and a command to run. */
    fun hostProblem(w: Words, name: String, error: String?): String = when {
        error?.contains("no gate key", ignoreCase = true) == true -> w.s(R.string.helpers_host_no_key, placeName(w, name))
        else -> w.s(R.string.helpers_host_unreachable, placeName(w, name))
    }

    /**
     * The start sheet's Where line when no computer answers. A phone runs no helper itself, so when this phone is the only place
     * listed the way on is to pair a computer; otherwise the first paired computer's problem is named, not this phone's.
     */
    fun noComputer(w: Words, hosts: List<HqClient.HelperHost>): String {
        val computer = hosts.firstOrNull { it.name !in THIS_PHONE } ?: return w.s(R.string.helpers_pair_a_computer)
        return hostProblem(w, computer.name, computer.error)
    }

    private val THIS_PHONE = setOf("", "local", "native")

    /**
     * True when the start sheet should walk the owner through pairing a computer instead of asking where to run: HQ listed
     * its places and none is a paired computer. A paired computer that is only offline is not this case.
     */
    fun needsComputer(hosts: List<HqClient.HelperHost>?): Boolean =
        !hosts.isNullOrEmpty() && hosts.none { it.reachable } && hosts.all { it.name in THIS_PHONE }

    /**
     * What the owner runs on the computer: the signed install script, then `hq bridge pair --setup`, which shows the code the
     * phone scans. A build published from another repository changes this line along with the script's address.
     */
    const val PAIR_COMMAND = "curl -fsSL https://agent-hq.online/install.sh | bash -s -- --pair"

    fun title(w: Words, row: HelperRow): String {
        val job = listOf(row.label, row.goal, row.taskTitle).firstOrNull { !it.isNullOrBlank() }?.trim()?.lineSequence()?.first()?.trim()
        return job?.let { if (it.length > MAX_TITLE) it.take(MAX_TITLE).trimEnd() + "…" else it } ?: w.s(R.string.helpers_default_title, agentName(w, row.harness))
    }

    fun state(row: HelperRow): HelperState = when {
        row.status == "exited" -> HelperState.Finished
        row.status == "stopped" -> HelperState.Stopped
        row.status == "orphaned" -> HelperState.Lost
        !row.reachable -> HelperState.Offline
        row.alive == false -> HelperState.Lost
        row.agentStatus == "blocked" -> HelperState.NeedsOk
        row.agentStatus == "working" -> HelperState.Working
        else -> HelperState.Ready
    }

    fun stateWords(w: Words, state: HelperState, own: Boolean = false): String = when {
        own && state == HelperState.Working -> w.s(R.string.helpers_own_working)
        own && state == HelperState.Finished -> w.s(R.string.helpers_own_done)
        else -> w.s(when (state) {
            HelperState.NeedsOk -> R.string.helpers_needs_your_ok
            HelperState.Working -> R.string.helpers_state_working
            HelperState.Ready -> R.string.helper_state_ready
            HelperState.Offline -> R.string.helpers_state_offline
            HelperState.Lost -> R.string.helpers_state_lost
            HelperState.Finished -> R.string.helpers_finished
            HelperState.Stopped -> R.string.helper_state_stopped
        })
    }

    /** A UTC time as HQ writes it: SQLite's "2026-11-03 07:43:52", or RFC 3339 with an offset (the edge worker's runs). */
    private fun parse(time: String): LocalDateTime? = example.hq.spike.data.Instants.parseUtc(time)

    /** "12 min", "2 h", "3 d": how long ago, with no "ago", for the line under a title. */
    fun span(w: Words, from: String, now: LocalDateTime): String? {
        val then = parse(from) ?: return null
        val minutes = Duration.between(then, now).toMinutes().coerceAtLeast(0)
        return when {
            minutes < 1 -> w.s(R.string.age_just_now)
            minutes < 60 -> w.s(R.string.helpers_span_min, minutes.toString())
            minutes < 24 * 60 -> w.s(R.string.helpers_span_h, (minutes / 60).toString())
            else -> w.s(R.string.helpers_span_d, (minutes / (24 * 60)).toString())
        }
    }

    private fun ago(w: Words, from: String, now: LocalDateTime): String = span(w, from, now)?.let { if (it == w.s(R.string.age_just_now)) it else w.s(R.string.helpers_ago, it) }.orEmpty()

    /** When a helper was last seen; the oldest rows never had a last-seen time (the server sends null), so they fall back to when they started. */
    fun lastSeen(row: HelperRow): String = row.lastSeenAt.takeIf { parse(it) != null } ?: row.createdAt

    /** The last words on a helper's screen worth showing: no box drawing, no empty lines. */
    fun lastWords(lines: List<String>): String? =
        lines.asReversed().map { cleanLine(it) }.firstOrNull { l -> l.any(Char::isLetter) && l.length > 2 }?.let { if (it.length > MAX_LINE) it.take(MAX_LINE).trimEnd() + "…" else it }

    /** What a label field may hold while it is typed: the first line, no control characters, at most 80 characters. A trailing space stays, or words could not be typed. */
    fun typedLabel(text: String): String {
        val first = text.trimStart().lineSequence().firstOrNull().orEmpty().filter { !it.isISOControl() }
        val cut = first.codePointCount(0, first.length).takeIf { it > MAX_TITLE }?.let { first.offsetByCodePoints(0, MAX_TITLE) } ?: first.length
        return first.substring(0, cut)
    }

    /** A label the server accepts: [typedLabel] with its ends trimmed. */
    fun oneLineLabel(text: String): String = typedLabel(text).trim()

    fun cleanLine(line: String): String = line.replace(BOX, " ").replace(Regex("\\s+"), " ").trim()

    private val FRAME = Regex("[\\u2500-\\u257F]")
    private val SPACES = Regex("\\s+")
    private val OPTION_LINE = Regex("^(?:[\\u276F\\u203A>\\u2192\\u25B6\\u25B8\\u279C\\u25CF]\\s*)?\\d+[.)]\\s+\\S")
    private val POINTER = Regex("^(?:[\\u276F\\u203A>\\u2192\\u25B6\\u25B8\\u279C]|[\\u25CF\\u25CB](?=\\s*\\d+[.)]))\\s*(.+)$")
    private val PLAN = Regex("(?i)would you like to proceed|ready to code|exit plan|\\bplan\\b")
    /** A trailing "(y)", "(esc)" or "(shift+tab)": what the key is, not a condition on the choice. */
    private val KEY_HINT = Regex("(?i)\\s*\\((?:[a-z]|esc|enter|tab|shift\\+tab|ctrl\\+\\w|alt\\+\\w)\\)\\s*$")
    private val HINT_LINE = Regex("(?i)^(?:esc|enter|tab|press|use|\\u2191|ctrl)\\b")
    private const val CONTEXT = 10
    private const val WINDOW = 28
    private const val MAX_AFTER_QUESTION = 12
    private const val INLINE_DETAIL_LINES = 3
    private const val INLINE_CHARS = 120
    /** How long an answered dialog stays hidden before it may be shown again, in case the key did nothing. */
    const val ANSWERED_GRACE_MS = 10_000L

    /** What a plain "yes, once" is called by the agents HQ drives; anything else (for the session, always, and so on) is not once. */
    private val ONCE = setOf("yes", "yes, proceed", "yes, allow once", "allow", "allow once", "approve", "accept", "ok", "proceed", "continue")

    /** The last lines of the screen with box lines removed but the cursor kept, each with how far it was indented; no blanks. */
    private fun indented(screen: List<String>, n: Int = WINDOW): List<Pair<Int, String>> =
        screen.map { it.replace(FRAME, " ") }.filter { it.isNotBlank() }.map { raw -> raw.takeWhile { it == ' ' }.length to raw.replace(SPACES, " ").trim() }.takeLast(n)

    /** The last lines of the screen with box lines removed but the cursor kept, one per line, no blanks. */
    fun tail(screen: List<String>, n: Int = WINDOW): List<String> = indented(screen, n).map { it.second }

    /** Reads the dialog out of the end of the screen; null when nothing there asks anything. */
    fun blocked(screen: List<String>): BlockedPrompt? {
        val lines = tail(screen)
        val qi = lines.indexOfLast { QUESTION.containsMatchIn(it) }
        // A dialog has a question and a few options under it; a question with a screenful of output after it is old.
        if (qi < 0 || lines.size - qi > MAX_AFTER_QUESTION) return null
        val q = cleanLine(lines[qi])
        val from = (qi - CONTEXT).coerceAtLeast(0)
        val block = lines.subList(from, lines.size)
        val detail = lines.subList(from, qi).map { cleanLine(it) }.filter { it.isNotEmpty() }
        val core = lines.size - qi
        fun prompt(kind: AskKind, subject: String? = null) = BlockedPrompt(kind, subject, q, block, detail, core)
        if (PLAN.containsMatchIn(q)) return prompt(AskKind.Plan)
        EDIT.find(q)?.let { return prompt(AskKind.Edit, it.groupValues[1]) }
        CREATE.find(q)?.let { return prompt(AskKind.Create, it.groupValues[1]) }
        if (COMMAND.containsMatchIn(q)) return prompt(AskKind.Command)
        return prompt(AskKind.Other)
    }

    /** The file name of a path as the dialog wrote it. */
    fun fileName(ask: BlockedPrompt): String = ask.subject.orEmpty().substringAfterLast('/')

    /**
     * Whether [now] is the dialog [shown] was. A notice carries only the end of the screen, so the shorter of the two is compared
     * against the same number of lines at the end of the other, and that must cover the question and the options.
     */
    fun sameDialog(shown: BlockedPrompt, now: BlockedPrompt): Boolean {
        val n = minOf(shown.block.size, now.block.size)
        return n >= shown.core && n >= now.core && shown.block.takeLast(n) == now.block.takeLast(n)
    }

    /** Whether everything the owner needs to decide is on the card itself; otherwise the card opens the helper's own page. */
    fun canAnswerInline(ask: BlockedPrompt): Boolean = when (ask.kind) {
        AskKind.Plan -> false
        AskKind.Command -> ask.detail.isNotEmpty() && ask.detail.size <= INLINE_DETAIL_LINES && ask.detail.sumOf { it.length } <= INLINE_CHARS
        AskKind.Edit, AskKind.Create -> !ask.subject.isNullOrBlank() && ask.subject.length <= INLINE_CHARS
        AskKind.Other -> ask.question.length <= INLINE_CHARS
    }

    /**
     * The option the terminal's cursor is on, with the lines that continue it (indented further than the option's own text),
     * or null when the screen shows no cursor.
     */
    fun highlightedOption(screen: List<String>): String? {
        val lines = indented(screen)
        val at = lines.indexOfLast { POINTER.containsMatchIn(it.second) }
        if (at < 0) return null
        val (indent, line) = lines[at]
        val glyph = line.takeWhile { !it.isLetterOrDigit() && !it.isWhitespace() }.length
        val textStart = indent + glyph + 1
        val head = POINTER.find(line)!!.groupValues[1].trim()
        val more = lines.drop(at + 1).takeWhile { (i, l) -> i > textStart && !OPTION_LINE.containsMatchIn(l) && !HINT_LINE.containsMatchIn(l) && !QUESTION.containsMatchIn(l) }.take(2).map { it.second }
        return (listOf(head) + more).joinToString(" ")
    }

    private fun normalOption(option: String): String =
        option.replace(Regex("^\\d+[.)]\\s*"), "").replace(KEY_HINT, "").replace(Regex("[.!\\s]+$"), "").replace(Regex("[\\u2018\\u2019]"), "'").lowercase().trim()

    /**
     * Enter takes whatever is highlighted, so "Allow once" is only safe when the cursor can be read and sits on one of the short
     * list of plain yeses. With no cursor to read, nothing says which choice Enter takes, so it is not safe.
     */
    fun enterMeansOnce(screen: List<String>): Boolean = highlightedOption(screen)?.let { normalOption(it) in ONCE } == true

    /** The checks run on a fresh read of the screen just before an answer is sent. [live] is whether that read came from the terminal itself. */
    fun checkAnswer(shown: BlockedPrompt?, screen: List<String>, key: String, live: Boolean = true): AnswerCheck {
        val now = blocked(screen)
        return when {
            !live -> AnswerCheck.NotLive
            shown == null || now == null -> AnswerCheck.NoPrompt
            !sameDialog(shown, now) -> AnswerCheck.Changed
            key == "enter" && highlightedOption(screen) == null -> AnswerCheck.NoCursor
            key == "enter" && !enterMeansOnce(screen) -> AnswerCheck.BroadYes
            else -> AnswerCheck.Ok
        }
    }

    /** The dialog alert of helper [id], if HQ raised one: its answer goes through the server, not through the screen. */
    fun decisionFor(rows: List<HelperRow>, id: String): HqClient.HelperDecision? = rows.firstOrNull { it.id == id }?.decision

    /** What to tell the owner when a check refuses the key, in the words used on every screen that answers a helper. */
    fun checkWords(w: Words, check: AnswerCheck): String = when (check) {
        AnswerCheck.Ok -> ""
        AnswerCheck.NotLive -> w.s(R.string.helpers_check_not_live)
        AnswerCheck.NoPrompt, AnswerCheck.Changed -> w.s(R.string.helpers_check_changed)
        AnswerCheck.BroadYes, AnswerCheck.NoCursor -> w.s(R.string.helpers_check_broad)
    }

    /** The lines HQ's session supervisor puts around a screen excerpt (`OUTPUT_OPEN` and `OUTPUT_CLOSE` in its Rust source). */
    const val OUTPUT_OPEN = "----- session output -----"
    const val OUTPUT_CLOSE = "----- end session output -----"

    /** The lines a notice quotes from the helper's screen, between its output markers; empty when it quotes none. */
    fun excerptLines(description: String): List<String> {
        val lines = description.lines()
        val from = lines.indexOfFirst { it.trim() == OUTPUT_OPEN }
        val to = lines.indexOfFirst { it.trim() == OUTPUT_CLOSE }
        return if (from >= 0 && to > from) lines.subList(from + 1, to) else emptyList()
    }

    /**
     * Asks the owner has already answered stay hidden until the screen shows a different dialog or none, or until [ANSWERED_GRACE_MS]
     * have passed with the same dialog still up (the key may have done nothing, and the owner must not be left with no buttons).
     */
    fun visibleAsks(asks: Map<String, BlockedPrompt>, answered: MutableMap<String, Pair<List<String>, Long>>, nowMs: Long): Map<String, BlockedPrompt> {
        answered.keys.retainAll(asks.keys)
        answered.entries.removeAll { (id, mark) -> asks[id]?.block != mark.first || nowMs - mark.second >= ANSWERED_GRACE_MS }
        return asks.filterKeys { it !in answered }
    }

    /** The end of the screen as the helper drew it, cursor included, for the owner to read exactly what it asked. */
    fun rawTail(screen: List<String>, n: Int): List<String> = tail(screen, n)

    fun askTitle(w: Words, agent: String, ask: BlockedPrompt): String = when (ask.kind) {
        AskKind.Edit -> w.s(R.string.helpers_ask_title_edit, agent)
        AskKind.Create -> w.s(R.string.helpers_ask_title_create, agent)
        AskKind.Command -> w.s(R.string.helpers_ask_title_command, agent)
        AskKind.Plan -> w.s(R.string.helpers_ask_title_plan, agent)
        AskKind.Other -> w.s(R.string.helpers_ask_title_other, agent)
    }

    fun askLine(w: Words, ask: BlockedPrompt): String = when (ask.kind) {
        AskKind.Edit -> w.s(R.string.helpers_ask_line_edit, ask.subject.toString())
        AskKind.Create -> w.s(R.string.helpers_ask_line_create, ask.subject.toString())
        AskKind.Command -> w.s(R.string.helpers_ask_line_command)
        AskKind.Plan -> w.s(R.string.helpers_ask_line_plan)
        AskKind.Other -> ask.question
    }

    /** [askLine] and, for a command, the lines it ran: for a place with no room to draw them apart, such as a notification. */
    fun askText(w: Words, ask: BlockedPrompt): String = askLine(w, ask) + if (ask.kind == AskKind.Command && ask.detail.isNotEmpty()) ":\n" + ask.detail.joinToString("\n") else ""

    /** The Context-free English of [askText] for InboxWords, which has no Words yet; it goes when that file takes one. */
    fun askText(ask: BlockedPrompt): String = when (ask.kind) {
        AskKind.Edit -> "Wants to edit one file: ${ask.subject}"
        AskKind.Create -> "Wants to create one file: ${ask.subject}"
        AskKind.Command -> "Wants to run a command"
        AskKind.Plan -> "Wants your OK on its plan"
        AskKind.Other -> ask.question
    } + if (ask.kind == AskKind.Command && ask.detail.isNotEmpty()) ":\n" + ask.detail.joinToString("\n") else ""

    fun allowHint(w: Words, ask: BlockedPrompt): String = w.s(when (ask.kind) {
        AskKind.Edit -> R.string.helpers_allow_hint_edit
        AskKind.Create -> R.string.helpers_allow_hint_create
        AskKind.Command -> R.string.helpers_allow_hint_command
        AskKind.Plan -> R.string.helpers_allow_hint_plan
        AskKind.Other -> R.string.helpers_allow_hint_other
    })

    fun ifAllowed(w: Words, ask: BlockedPrompt, where: String): String = when (ask.kind) {
        AskKind.Edit -> w.s(R.string.helpers_if_allowed_edit, where)
        AskKind.Create -> w.s(R.string.helpers_if_allowed_create, where)
        AskKind.Command -> w.s(R.string.helpers_if_allowed_command, where)
        AskKind.Plan -> w.s(R.string.helpers_if_allowed_plan, where)
        AskKind.Other -> w.s(R.string.helpers_if_allowed_other)
    }

    fun ifNotNow(w: Words): String = w.s(R.string.helpers_if_not_now)

    fun card(w: Words, row: HelperRow, ask: BlockedPrompt?, now: LocalDateTime, lastWords: String? = null): HelperCard {
        val state = state(row)
        val agent = agentName(w, row.harness)
        val where = placeName(w, row.host)
        val line = when (state) {
            HelperState.NeedsOk -> ask?.let { askLine(w, it) } ?: w.s(R.string.helpers_line_waiting)
            HelperState.Working -> row.detail?.takeIf { it.isNotBlank() }?.let { cleanLine(it) } ?: lastWords ?: w.s(R.string.helpers_line_working)
            HelperState.Ready -> w.s(R.string.helpers_line_ready)
            HelperState.Offline -> hostProblem(w, row.host, row.detail).removeSuffix(".")
            HelperState.Lost -> w.s(R.string.helpers_line_lost)
            HelperState.Finished -> w.s(R.string.helpers_line_finished, ago(w, lastSeen(row), now)).trim()
            HelperState.Stopped -> w.s(R.string.helpers_line_stopped, ago(w, lastSeen(row), now)).trim()
        }
        val time = if (state == HelperState.Working || state == HelperState.NeedsOk) span(w, row.createdAt, now).orEmpty() else ago(w, lastSeen(row), now)
        return HelperCard(row.id, title(w, row), agent, where, state, line, time, ask.takeIf { state == HelperState.NeedsOk }, row.id, isGeneric(row), folderOf(row.cwd))
    }

    /** A run of HQ's own edge worker on this phone, as a card beside the other helpers. Its id starts with [EDGE_PREFIX]. */
    fun edgeCard(w: Words, run: HqClient.EdgeRun, now: LocalDateTime): HelperCard {
        val state = when (run.status) {
            "pending" -> HelperState.Working
            "answered" -> HelperState.Finished
            else -> HelperState.Stopped
        }
        val task = run.displayId ?: w.s(R.string.helpers_a_task)
        val line = when (state) {
            HelperState.Working -> w.s(R.string.helpers_edge_working, task)
            HelperState.Finished -> listOfNotNull(task, run.taskStatus?.let { taskStatusWords(w, it) }).joinToString(" · ")
            else -> w.s(R.string.helpers_edge_stopped, task)
        }
        val time = if (state == HelperState.Working) span(w, run.startedAt, now).orEmpty() else ago(w, run.finishedAt ?: run.startedAt, now)
        return HelperCard(EDGE_PREFIX + run.askId, run.title ?: task, EDGE_AGENT, w.s(R.string.helpers_this_phone), state, line, time, null, "")
    }

    private fun taskStatusWords(w: Words, status: String) = w.s(when (status) {
        "ready_for_review" -> R.string.helpers_task_ready_review
        "blocked" -> R.string.helpers_task_blocked
        "complete" -> R.string.helpers_task_complete
        "in_progress" -> R.string.helpers_task_in_progress
        else -> R.string.helpers_task_to_do
    })

    const val EDGE_PREFIX = "edge:"
    private const val EDGE_AGENT = "HQ"

    /** The order on the list: waiting for the owner first, then working, then the rest, newest first within each. */
    val order: Comparator<HelperCard> = compareBy<HelperCard> {
        when (it.state) {
            HelperState.NeedsOk -> 0
            HelperState.Working -> 1
            HelperState.Ready -> 2
            HelperState.Offline, HelperState.Lost -> 3
            HelperState.Finished, HelperState.Stopped -> 4
        }
    }

    /** Why "Follow in chat" did not open a chat: [status] is the HTTP status HQ answered with, 0 when it did not answer, null when it answered with no chat. */
    fun followFailed(w: Words, status: Int?): String = when (status) {
        404 -> w.s(R.string.helpers_follow_gone)
        0, 503 -> w.s(R.string.helpers_follow_timeout)
        else -> w.s(R.string.helpers_open_chat_failed)
    }
}
