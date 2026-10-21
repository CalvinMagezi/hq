package example.hq.spike.ui.helpers

import example.hq.spike.data.HqClient.HelperRow
import java.time.LocalDateTime
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HelperWordsTest {
    private val now = LocalDateTime.of(2026, 10, 21, 12, 0, 0)

    private fun row(
        status: String = "running", agent: String = "working", label: String = "", goal: String? = null, harness: String = "claude-code", host: String = "mac-mini",
        reachable: Boolean = true, alive: Boolean? = true, detail: String? = null, seen: String = "2026-10-21 11:58:00", created: String = "2026-10-21 11:48:00", task: String? = null,
    ) = HelperRow("h1", harness, label, host, "/Users/x/hq-orch-probe", status, agent, seen, created, goal, task, alive, reachable, detail, false)

    @Test fun theTitleIsTheJobAndNeverTheFolder() {
        assertEquals("Fix the backup script", HelperWords.title(row(label = "Fix the backup script", goal = "ignored")))
        assertEquals("Print ready then done", HelperWords.title(row(goal = "Print ready then done")))
        assertEquals("Phone sign-in", HelperWords.title(row(task = "Phone sign-in")))
        assertEquals("Claude helper", HelperWords.title(row()))
        assertEquals("Codex helper", HelperWords.title(row(harness = "codex")))
    }

    @Test fun aLongGoalIsCutAndOnlyItsFirstLineIsUsed() {
        val long = "word ".repeat(40).trim() + "\nsecond line"
        val title = HelperWords.title(row(goal = long))
        assertTrue(title.length <= 81)
        assertFalse(title.contains("second"))
        assertTrue(title.endsWith("…"))
    }

    @Test fun theStateComesFromStatusAndWhatTheComputerSaid() {
        assertEquals(HelperState.NeedsOk, HelperWords.state(row(agent = "blocked")))
        assertEquals(HelperState.Working, HelperWords.state(row(agent = "working")))
        assertEquals(HelperState.Ready, HelperWords.state(row(agent = "idle")))
        assertEquals(HelperState.Offline, HelperWords.state(row(agent = "unknown", reachable = false, alive = null)))
        assertEquals(HelperState.Lost, HelperWords.state(row(alive = false)))
        assertEquals(HelperState.Finished, HelperWords.state(row(status = "exited")))
        assertEquals(HelperState.Stopped, HelperWords.state(row(status = "stopped")))
        assertEquals(HelperState.Lost, HelperWords.state(row(status = "orphaned")))
    }

    @Test fun aComputerThatIsDownIsNeverFinished() {
        val s = HelperWords.state(row(agent = "blocked", reachable = false, alive = null))
        assertEquals(HelperState.Offline, s)
        assertEquals("Computer offline", HelperWords.stateWords(s))
    }

    @Test fun namesAreInTheOwnersWords() {
        assertEquals("Claude", HelperWords.agentName("claude-code"))
        assertEquals("Codex", HelperWords.agentName("codex"))
        assertEquals("Open Code", HelperWords.agentName("open-code"))
        assertEquals("your mac-mini", HelperWords.placeName("mac-mini"))
        assertEquals("this phone", HelperWords.placeName("local"))
        assertEquals("this phone", HelperWords.placeName("native"))
    }

    private val editScreen = listOf(
        "╭────────────────────────────────────╮",
        "│ Edit file                          │",
        "│ Chat.dc.html                       │",
        "╰────────────────────────────────────╯",
        "Do you want to make this edit to Chat.dc.html?",
        "❯ 1. Yes",
        "  2. Yes, allow all edits during this session",
        "  3. No, and tell Claude what to do differently",
    )

    @Test fun anEditQuestionNamesTheFile() {
        val ask = HelperWords.blocked(editScreen)!!
        assertEquals(AskKind.Edit, ask.kind)
        assertEquals("Chat.dc.html", ask.subject)
        assertEquals("Claude wants to edit a file", HelperWords.askTitle("Claude", ask))
        assertEquals("Wants to edit one file: Chat.dc.html", HelperWords.askLine(ask))
        assertEquals("Allow once lets it change this file one time.", HelperWords.allowHint(ask))
        assertEquals("It changes this one file on your mac-mini, then carries on.", HelperWords.ifAllowed(ask, "your mac-mini"))
    }

    @Test fun aPathIsReducedToItsFileName() {
        val ask = HelperWords.blocked(listOf("Do you want to edit /Users/x/work/src/main.rs?"))!!
        assertEquals("main.rs", ask.subject)
    }

    @Test fun aCreateQuestionAndACommandQuestionAreTold() {
        assertEquals(AskKind.Create, HelperWords.blocked(listOf("Do you want to create notes.md?"))!!.kind)
        val run = HelperWords.blocked(listOf("Bash command", "  rm -rf build", "Do you want to proceed?"))!!
        assertEquals(AskKind.Command, run.kind)
        assertEquals("Claude wants to run a command", HelperWords.askTitle("Claude", run))
    }

    @Test fun anyOtherQuestionIsShownAsItWasAsked() {
        val ask = HelperWords.blocked(listOf("Which colour do you prefer?"))!!
        assertEquals(AskKind.Other, ask.kind)
        assertEquals("Which colour do you prefer?", HelperWords.askLine(ask))
        assertEquals("Codex has a question", HelperWords.askTitle("Codex", ask))
    }

    @Test fun aScreenThatAsksNothingGivesNoQuestion() {
        assertNull(HelperWords.blocked(listOf("● Updating the build files", "  ⎿ Read build.gradle.kts")))
        assertNull(HelperWords.blocked(emptyList()))
    }

    @Test fun onlyTheEndOfTheScreenCounts() {
        val old = listOf("Do you want to edit old.txt?") + List(20) { "output line $it" }
        assertNull(HelperWords.blocked(old))
    }

    @Test fun theLastWordsHaveNoBoxDrawingOrBullets() {
        assertEquals("Updating the build files", HelperWords.lastWords(listOf("  ", "● Updating the build files", "")))
        assertEquals("Read build.gradle.kts", HelperWords.lastWords(listOf("x", "  ⎿ Read build.gradle.kts")))
        assertNull(HelperWords.lastWords(listOf("──────", "  ")))
    }

    @Test fun aBlockedCardShowsTheQuestionAndAnAnswerableOne() {
        val ask = HelperWords.blocked(editScreen)
        val c = HelperWords.card(row(agent = "blocked", label = "Move the build"), ask, now)
        assertEquals(HelperState.NeedsOk, c.state)
        assertEquals("Wants to edit one file: Chat.dc.html", c.line)
        assertEquals("12 min", c.time)
        assertEquals(ask, c.ask)
    }

    @Test fun aBlockedHelperWithNoReadableQuestionOffersNoAnswer() {
        val c = HelperWords.card(row(agent = "blocked"), null, now)
        assertEquals("Waiting for your answer", c.line)
        assertNull(c.ask)
    }

    @Test fun aWorkingCardPrefersWhatTheComputerSaidThenTheScreen() {
        assertEquals("Updating the build files", HelperWords.card(row(detail = "Updating the build files"), null, now, "from screen").line)
        assertEquals("from screen", HelperWords.card(row(), null, now, "from screen").line)
        assertEquals("Working", HelperWords.card(row(), null, now).line)
    }

    @Test fun finishedAndOfflineCardsSayWhenAndWhy() {
        val done = HelperWords.card(row(status = "exited", agent = "done", seen = "2026-10-21 09:55:00"), null, now)
        assertEquals("Finished 2 h ago", done.line)
        assertEquals("2 h ago", done.time)
        val off = HelperWords.card(row(reachable = false, alive = null, agent = "unknown"), null, now)
        assertEquals("HQ cannot reach your mac-mini right now", off.line)
    }

    @Test fun theListPutsWhatNeedsYouFirstAndKeepsTheServersOrderWithinAGroup() {
        val rows = listOf(
            row(status = "exited", agent = "done").copy(id = "old"), row(agent = "working").copy(id = "w"), row(agent = "blocked").copy(id = "b"),
            row(status = "stopped").copy(id = "s"), row(agent = "idle").copy(id = "r"),
        )
        val cards = rows.map { HelperWords.card(it, null, now) }.sortedWith(HelperWords.order)
        assertEquals(listOf("b", "w", "r", "old", "s"), cards.map { it.id })
    }

    @Test fun theFiltersCountTheRightCards() {
        val ui = HelpersUi(
            loaded = true, now = now,
            rows = listOf(row(agent = "blocked").copy(id = "a"), row().copy(id = "b"), row(status = "exited").copy(id = "c"), row(status = "stopped").copy(id = "d"), row(agent = "idle").copy(id = "e")),
        )
        assertEquals(5, ui.count(HelperFilter.All))
        assertEquals(1, ui.count(HelperFilter.Needs))
        assertEquals(1, ui.count(HelperFilter.Working))
        assertEquals(2, ui.count(HelperFilter.Finished))
    }

    @Test fun anArchivedHelperIsNotListed() {
        val ui = HelpersUi(loaded = true, now = now, rows = listOf(row().copy(archived = true), row().copy(id = "x")))
        assertEquals(listOf("x"), ui.cards.map { it.id })
    }

    @Test fun spansReadInMinutesHoursAndDays() {
        assertEquals("just now", HelperWords.span("2026-10-21 11:59:40", now))
        assertEquals("12 min", HelperWords.span("2026-10-21 11:48:00", now))
        assertEquals("3 h", HelperWords.span("2026-10-21 08:30:00", now))
        assertEquals("2 d", HelperWords.span("2026-10-19 11:00:00", now))
        assertNull(HelperWords.span("not a time", now))
    }

    private fun bash(command: String, cursorOn: Int = 1) = listOf(
        "╭──────────────────────────────╮",
        "│ Bash command                 │",
        "│   $command                   │",
        "╰──────────────────────────────╯",
        "Do you want to proceed?",
        (if (cursorOn == 1) "❯ " else "  ") + "1. Yes",
        (if (cursorOn == 2) "❯ " else "  ") + "2. Yes, and don't ask again for similar commands",
        (if (cursorOn == 3) "❯ " else "  ") + "3. No, and tell Claude what to do differently",
    )

    @Test fun twoPromptsWithTheSameQuestionButADifferentCommandAreNotTheSame() {
        val a = HelperWords.blocked(bash("rm -rf build"))!!
        val b = HelperWords.blocked(bash("rm -rf /"))!!
        assertEquals(a.question, b.question)
        assertFalse(a.block == b.block)
        assertEquals(AnswerCheck.Changed, HelperWords.checkAnswer(a, bash("rm -rf /"), "enter"))
        assertEquals(AnswerCheck.Ok, HelperWords.checkAnswer(a, bash("rm -rf build"), "enter"))
    }

    @Test fun theCommandIsShownToTheOwner() {
        val ask = HelperWords.blocked(bash("rm -rf build"))!!
        assertEquals(AskKind.Command, ask.kind)
        assertTrue(ask.detail!!.contains("rm -rf build"))
        assertEquals("Wants to run: rm -rf build", HelperWords.askLine(ask))
    }

    @Test fun theCursorMovingMakesItADifferentDialog() {
        val ask = HelperWords.blocked(bash("ls"))!!
        assertEquals(AnswerCheck.Changed, HelperWords.checkAnswer(ask, bash("ls", cursorOn = 2), "esc"))
    }

    @Test fun onlyTheShortListOfPlainYesesCountsAsOnce() {
        listOf("Yes", "1. Yes", "Yes, proceed (y)", "Allow", "Allow once", "OK").forEach {
            assertTrue(it, HelperWords.enterMeansOnce(listOf("Do you want to proceed?", "❯ $it", "  No")))
        }
        listOf(
            "1. Yes, and auto-accept edits", "Yes, clear context (45% used) and auto-accept edits (shift+tab)", "Yes, and bypass permissions",
            "Yes, and remember this choice", "Yes, and don\u2019t ask again", "Yes, and don't ask again", "Yes, allow all edits during this session", "No, and tell Claude what to do differently",
        ).forEach { assertFalse(it, HelperWords.enterMeansOnce(listOf("Do you want to proceed?", "❯ $it", "  No"))) }
    }

    @Test fun aQualifierOnTheNextLineStillCounts() {
        val screen = listOf("Do you want to proceed?", "❯ 1. Yes, allow Claude to edit its own settings", "     for this session", "  2. No")
        assertEquals("1. Yes, allow Claude to edit its own settings for this session", HelperWords.highlightedOption(screen))
        assertFalse(HelperWords.enterMeansOnce(screen))
    }

    @Test fun optionsWithNoCursorToReadAreNotSafeForEnter() {
        val noCursor = listOf("Do you want to proceed?", "  1. Yes", "  2. Yes, and don't ask again", "  3. No")
        assertFalse(HelperWords.enterMeansOnce(noCursor))
        assertEquals(AnswerCheck.NoCursor, HelperWords.checkAnswer(HelperWords.blocked(noCursor), noCursor, "enter"))
        assertEquals(AnswerCheck.Ok, HelperWords.checkAnswer(HelperWords.blocked(noCursor), noCursor, "esc"))
        assertTrue(HelperWords.enterMeansOnce(listOf("Which colour do you prefer?")))
    }

    @Test fun otherCursorGlyphsAreRead() {
        assertEquals("1. Yes", HelperWords.highlightedOption(listOf("Do you want to proceed?", "▶ 1. Yes", "  2. No")))
        assertEquals("1. Yes", HelperWords.highlightedOption(listOf("Do you want to proceed?", "● 1. Yes", "  2. No")))
        assertNull(HelperWords.highlightedOption(listOf("● Updating the build files", "  ⎿ Read it")))
    }

    @Test fun aPlanPromptIsAPlanNotACommand() {
        val ask = HelperWords.blocked(listOf("Here is the plan: move the build.", "Would you like to proceed?", "❯ 1. Yes, and auto-accept edits", "  2. No"))!!
        assertEquals(AskKind.Plan, ask.kind)
        assertEquals("Claude has a plan to approve", HelperWords.askTitle("Claude", ask))
        assertFalse(HelperWords.enterMeansOnce(listOf("Would you like to proceed?", "❯ 1. Yes, and auto-accept edits", "  2. No")))
    }

    @Test fun aScreenThatNoLongerAsksIsNoPrompt() {
        val ask = HelperWords.blocked(bash("ls"))
        assertEquals(AnswerCheck.NoPrompt, HelperWords.checkAnswer(ask, listOf("● Running ls", "total 4"), "enter"))
        assertEquals(AnswerCheck.NoPrompt, HelperWords.checkAnswer(null, bash("ls"), "enter"))
    }

    @Test fun anAnsweredDialogStaysHiddenUntilTheScreenShowsAnother() {
        val one = HelperWords.blocked(bash("ls"))!!
        val two = HelperWords.blocked(bash("pwd"))!!
        val answered = mutableMapOf("h" to one.block)
        assertEquals(emptyMap<String, BlockedPrompt>(), HelperWords.visibleAsks(mapOf("h" to one), answered))
        assertEquals(mapOf("h" to two), HelperWords.visibleAsks(mapOf("h" to two), answered))
        assertTrue(answered.isEmpty())
        val again = mutableMapOf("h" to one.block)
        HelperWords.visibleAsks(emptyMap(), again)
        assertTrue(again.isEmpty())
    }

    @Test fun aLabelIsOneLineWithNoControlCharactersAndCutBetweenWholeCharacters() {
        assertEquals("Move the build", HelperWords.oneLineLabel("  Move the build\nthen test it"))
        assertEquals("a b", HelperWords.oneLineLabel("a\u0007 b"))
        val long = "\uD83D\uDE00".repeat(100)
        val cut = HelperWords.oneLineLabel(long)
        assertEquals(80, cut.codePointCount(0, cut.length))
        assertEquals("", HelperWords.oneLineLabel("\n\n"))
    }

    @Test fun aHelperWithNoNameIsGenericAndShowsItsFolder() {
        val nameless = row(label = "", goal = null, task = null).copy(cwd = "/Users/x/Documents/HQ/egress-probe 2026-10-19 1731/")
        val c = HelperWords.card(nameless, null, now)
        assertTrue(c.generic)
        assertEquals("Claude helper", c.title)
        assertEquals("egress-probe 2026-10-19 1731", c.folder)
        assertFalse(HelperWords.card(row(label = "Fix the backup"), null, now).generic)
        assertFalse(HelperWords.card(row(goal = "Print ready"), null, now).generic)
    }

    @Test fun aMissingKeyIsSaidInPlainWords() {
        val error = "host 'laptop' unreachable: no gate key at /data/user/0/example.hq.spike/files/hqhome/.hq/gate/laptop.key; make one with `hq host gate-key <host>`"
        assertEquals("This phone has no key to reach your laptop yet.", HelperWords.hostProblem("laptop", error))
        assertEquals("HQ cannot reach your laptop right now.", HelperWords.hostProblem("laptop", "connection refused"))
        assertEquals("HQ cannot reach your laptop right now.", HelperWords.hostProblem("laptop", null))
        val offline = HelperWords.card(row(agent = "unknown", reachable = false, alive = null, host = "laptop", detail = error), null, now)
        assertEquals("This phone has no key to reach your laptop yet", offline.line)
        assertEquals("HQ cannot reach your mac-mini right now", HelperWords.card(row(agent = "unknown", reachable = false, alive = null), null, now).line)
    }

    @Test fun aRowWithNoLastSeenTimeUsesWhenItStarted() {
        val old = row(status = "stopped", agent = "idle", seen = "", created = "2026-10-19 10:00:00")
        val c = HelperWords.card(old, null, now)
        assertEquals("Stopped 2 d ago", c.line)
        assertEquals("2 d ago", c.time)
        assertEquals("2026-10-19 10:00:00", HelperWords.lastSeen(old))
        assertEquals("2026-10-21 11:58:00", HelperWords.lastSeen(row()))
    }

    @Test fun aLastSeenTimeThatIsNotATimeIsIgnored() {
        assertEquals("2026-10-19 10:00:00", HelperWords.lastSeen(row(seen = "null", created = "2026-10-19 10:00:00")))
        assertEquals("2026-10-19 10:00:00", HelperWords.lastSeen(row(seen = "soon", created = "2026-10-19 10:00:00")))
    }

    @Test fun theRealListShapeWithNullTimesGivesAnAgeForEveryRow() {
        val json = """{"sessions":[
            {"id":"h1","harness":"claude-code","label":"","host":"laptop","cwd":"/Users/x/Documents/HQ/kola-check","status":"stopped","agent_status":"idle","last_seen_at":null,"created_at":"2026-09-24 16:10:41","goal":null,"alive":false,"reachable":true,"archived":false},
            {"id":"h2","harness":"codex","label":"pilot","host":"laptop","cwd":"/work/p","status":"exited","agent_status":"done","last_seen_at":"2026-10-19 17:32:03","created_at":"2026-10-19 17:31:22","goal":"Prove it","task":null,"alive":null,"reachable":false,"detail":"host 'laptop' unreachable: no gate key at /x","archived":false}]}"""
        val rows = example.hq.spike.data.helperRows(json)
        assertEquals("", rows[0].lastSeenAt)
        assertNull(rows[0].goal)
        assertNull(rows[1].alive)
        val cards = rows.map { HelperWords.card(it, null, now) }
        assertEquals("Stopped 26 d ago", cards[0].line)
        assertEquals("26 d ago", cards[0].time)
        assertEquals("Finished 1 d ago", cards[1].line)
    }
}
