package example.hq.spike.ui.helpers

import example.hq.spike.data.HqClient.HelperRow
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
)

/** The words the Helpers board uses, kept out of the screens so they can be tested. */
object HelperWords {
    private val BOX = Regex("[\\u2500-\\u257F\\u2580-\\u259F\\u25A0-\\u25FF\\u2700-\\u27BF\\u23BF\\u23FA\\u2733\\u273B]")
    private val QUESTION = Regex("\\?\\s*$")
    private val EDIT = Regex("(?i)\\b(?:edit|modify|change|update)\\s+(?:to\\s+)?(?:the\\s+|this\\s+)?(?:file\\s+)?[`'\"]?([\\w./~-]+\\.\\w+)")
    private val CREATE = Regex("(?i)\\b(?:create|write|make)\\s+(?:the\\s+|a\\s+|new\\s+)?(?:file\\s+)?[`'\"]?([\\w./~-]+\\.\\w+)")
    private val COMMAND = Regex("(?i)\\b(?:run|execute)\\b|\\bbash\\b|\\bproceed\\b|\\bcommand\\b")
    private const val MAX_TITLE = 80
    const val MAX_LINE = 140

    fun agentName(harness: String): String = when (harness) {
        "claude-code", "claude" -> "Claude"
        "codex" -> "Codex"
        "cursor" -> "Cursor"
        "opencode" -> "OpenCode"
        "copilot", "copilot-cli" -> "Copilot"
        "hermes" -> "Hermes"
        "openclaw" -> "OpenClaw"
        else -> harness.split('-', '_').filter { it.isNotBlank() }.joinToString(" ") { it.replaceFirstChar(Char::uppercase) }.ifBlank { "A helper" }
    }

    fun placeName(host: String): String = when (host) {
        "", "local", "native" -> "this phone"
        else -> "your $host"
    }

    /** The job, never a folder name: the label, else the goal, else the task it serves, else a plain name. */
    fun isGeneric(row: HelperRow): Boolean = listOf(row.label, row.goal, row.taskTitle).none { !it.isNullOrBlank() }

    /** The last part of the folder a helper ran in, without a trailing slash. */
    fun folderOf(cwd: String): String = cwd.trimEnd('/').substringAfterLast('/').trim()

    /** Why a computer cannot be reached, in the owner's words. The server's own text names a missing key file and a command to run. */
    fun hostProblem(name: String, error: String?): String = when {
        error?.contains("no gate key", ignoreCase = true) == true -> "This phone has no key to reach ${placeName(name)} yet."
        else -> "HQ cannot reach ${placeName(name)} right now."
    }

    fun title(row: HelperRow): String {
        val job = listOf(row.label, row.goal, row.taskTitle).firstOrNull { !it.isNullOrBlank() }?.trim()?.lineSequence()?.first()?.trim()
        return job?.let { if (it.length > MAX_TITLE) it.take(MAX_TITLE).trimEnd() + "…" else it } ?: "${agentName(row.harness)} helper"
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

    fun stateWords(state: HelperState): String = when (state) {
        HelperState.NeedsOk -> "Needs your OK"
        HelperState.Working -> "Working on it"
        HelperState.Ready -> "Ready"
        HelperState.Offline -> "Computer offline"
        HelperState.Lost -> "Lost contact"
        HelperState.Finished -> "Finished"
        HelperState.Stopped -> "Stopped"
    }

    private fun parse(time: String): LocalDateTime? = try { LocalDateTime.parse(time.replace(' ', 'T')) } catch (e: DateTimeParseException) { null }

    /** "12 min", "2 h", "3 d": how long ago, with no "ago", for the line under a title. */
    fun span(from: String, now: LocalDateTime): String? {
        val then = parse(from) ?: return null
        val minutes = Duration.between(then, now).toMinutes().coerceAtLeast(0)
        return when {
            minutes < 1 -> "just now"
            minutes < 60 -> "$minutes min"
            minutes < 24 * 60 -> "${minutes / 60} h"
            else -> "${minutes / (24 * 60)} d"
        }
    }

    private fun ago(from: String, now: LocalDateTime): String = span(from, now)?.let { if (it == "just now") it else "$it ago" }.orEmpty()

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

    /** What to tell the owner when a check refuses the key, in the words used on every screen that answers a helper. */
    fun checkWords(check: AnswerCheck): String = when (check) {
        AnswerCheck.Ok -> ""
        AnswerCheck.NotLive -> "Couldn't read its screen just now. Look again before you answer."
        AnswerCheck.NoPrompt, AnswerCheck.Changed -> "It asked something else just now. Open the helper and look again before you answer."
        AnswerCheck.BroadYes, AnswerCheck.NoCursor -> "Its highlighted choice would say yes for good, or cannot be read. Open the helper to pick the answer."
    }

    /** The lines a notice quotes from the helper's screen, between its output markers; empty when it quotes none. */
    fun excerptLines(description: String): List<String> {
        val lines = description.lines()
        val from = lines.indexOfFirst { it.trim() == "<hq-output>" }
        val to = lines.indexOfFirst { it.trim() == "</hq-output>" }
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

    fun askTitle(agent: String, ask: BlockedPrompt): String = when (ask.kind) {
        AskKind.Edit -> "$agent wants to edit a file"
        AskKind.Create -> "$agent wants to create a file"
        AskKind.Command -> "$agent wants to run a command"
        AskKind.Plan -> "$agent has a plan to approve"
        AskKind.Other -> "$agent has a question"
    }

    fun askLine(ask: BlockedPrompt): String = when (ask.kind) {
        AskKind.Edit -> "Wants to edit one file: ${ask.subject}"
        AskKind.Create -> "Wants to create one file: ${ask.subject}"
        AskKind.Command -> "Wants to run a command"
        AskKind.Plan -> "Wants your OK on its plan"
        AskKind.Other -> ask.question
    }

    /** [askLine] and, for a command, the lines it ran: for a place with no room to draw them apart, such as a notification. */
    fun askText(ask: BlockedPrompt): String = askLine(ask) + if (ask.kind == AskKind.Command && ask.detail.isNotEmpty()) ":\n" + ask.detail.joinToString("\n") else ""

    fun allowHint(ask: BlockedPrompt): String = when (ask.kind) {
        AskKind.Edit -> "Allow once lets it change this file one time."
        AskKind.Create -> "Allow once lets it create this file one time."
        AskKind.Command -> "Allow once lets it run this one command."
        AskKind.Plan -> "Allow once says yes to the plan as it stands."
        AskKind.Other -> "Allow once says yes to this one question."
    }

    fun ifAllowed(ask: BlockedPrompt, where: String): String = when (ask.kind) {
        AskKind.Edit -> "It changes this one file on $where, then carries on."
        AskKind.Create -> "It creates this one file on $where, then carries on."
        AskKind.Command -> "It runs this one command on $where, then carries on."
        AskKind.Plan -> "It goes ahead with the plan on $where, then carries on."
        AskKind.Other -> "It goes ahead with what it asked, then carries on."
    }

    const val IF_NOT_NOW = "It stops that step and waits. You can tell it what to do instead."

    fun card(row: HelperRow, ask: BlockedPrompt?, now: LocalDateTime, lastWords: String? = null): HelperCard {
        val state = state(row)
        val agent = agentName(row.harness)
        val where = placeName(row.host)
        val line = when (state) {
            HelperState.NeedsOk -> ask?.let(::askLine) ?: "Waiting for your answer"
            HelperState.Working -> row.detail?.takeIf { it.isNotBlank() }?.let { cleanLine(it) } ?: lastWords ?: "Working"
            HelperState.Ready -> "Waiting for your next instruction"
            HelperState.Offline -> hostProblem(row.host, row.detail).removeSuffix(".")
            HelperState.Lost -> "HQ lost contact with this helper"
            HelperState.Finished -> "Finished ${ago(lastSeen(row), now)}".trim()
            HelperState.Stopped -> "Stopped ${ago(lastSeen(row), now)}".trim()
        }
        val time = if (state == HelperState.Working || state == HelperState.NeedsOk) span(row.createdAt, now).orEmpty() else ago(lastSeen(row), now)
        return HelperCard(row.id, title(row), agent, where, state, line, time, ask.takeIf { state == HelperState.NeedsOk }, row.id, isGeneric(row), folderOf(row.cwd))
    }

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
}
