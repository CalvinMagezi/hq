package example.hq.spike.ui.notes

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import example.hq.spike.data.HqClient.NoteItem
import example.hq.spike.ui.chat.NoteSheet
import java.time.OffsetDateTime
import java.time.ZoneOffset

/** Fixed Notes screens for checking against the design with no hq: `--es fixture n-home`, `n-reader`, `n-search` and so on. */
fun notesFixture(name: String?): NotesUi? {
    fun ago(minutes: Long) = OffsetDateTime.now(ZoneOffset.UTC).minusMinutes(minutes).toString()
    val recent = listOf(
        NoteItem("Notebooks/Projects/Agent-HQ/Backup-Setup-2026-10-20.md", "Backup-Setup-2026-10-20", "", ago(180), false),
        NoteItem("Notebooks/Projects/Agent-HQ/Blind-Relay-Design-2026-10-20.md", "Blind-Relay-Design-2026-10-20", "", ago(1500), false),
        NoteItem("Notebooks/Inbox/Quick idea widget copy.md", "Quick idea: widget copy", "", ago(1600), false),
    )
    val pinned = listOf(
        NoteItem("Notebooks/Projects/Agent-HQ/Mobile-First-Roadmap.md", "Mobile-First-Roadmap", "", ago(1500), true),
        NoteItem("Notebooks/Projects/Agent-HQ/Phone-Cutover-Evidence-2026-10-20.md", "Phone-Cutover-Evidence-2026-10-20", "", ago(1500), true),
    )
    val folders = listOf("Notebooks/Inbox", "Notebooks/Projects", "Notebooks/Projects/Agent-HQ", "Notebooks/Daily", "Notebooks/Logs")
    val home = NotesUi(loaded = true, recent = recent, pinned = pinned, folders = folders)
    val body = """
        ---
        title: Mobile-First-Roadmap
        pinned: true
        ---
        This is where AgentHQ goes from here. Workstream C is a native view instead of the WebView.

        |Workstream|Status|
        |---|---|
        |A. First run|Planned|
        |C. Native view|Drawing|

        Test on the A32 first. Samsung may undo the battery setting after an update.
    """.trimIndent()
    val sheet = NoteSheet("Notebooks/Projects/Agent-HQ/Mobile-First-Roadmap.md", "Mobile-First-Roadmap", "Projects › Agent-HQ", body, false, null, pinned = true)
    val reader = ReaderPhase.Ready(sheet, "edited 1d ago")
    val longSheet = NoteSheet("Notebooks/Projects/Agent-HQ/Build-Plan-And-Cutover.md", "Build-Plan-And-Cutover", "Projects › Agent-HQ", longNote(), false, null)
    val longReader = ReaderPhase.Ready(longSheet, "edited 33m ago")
    if (name == "n-suite-sheet") {
        val cell = { t: String, f: Boolean, n: Boolean -> example.hq.spike.data.SheetCell(t, f, n, false) }
        example.hq.spike.ui.chat.blocks.SheetFixture.view = example.hq.spike.data.SheetView(
            "Stock",
            listOf(
                listOf(cell("item", false, false), cell("qty", false, false), cell("price", false, false), cell("total", false, false)),
                listOf(cell("Sugar", false, false), cell("40", false, true), cell("UGX 4,200", false, true), cell("UGX 168,000", true, true)),
                listOf(cell("Salt", false, false), cell("25", false, true), cell("UGX 1,800", false, true), cell("UGX 45,000", true, true)),
                listOf(cell("Total", false, false), cell("", false, false), cell("", false, false), cell("UGX 213,000", true, true)),
            ),
            0, 0, 4, 4, "Stock: 4 rows, 4 columns",
        )
        example.hq.spike.ui.chat.blocks.SheetFixture.tile = example.hq.spike.data.TileView("Stock value", "UGX 213,000")
        val body = "# Stock\n\n```tile from=\"[[Stock]]!D2:D3\" fn=sum label=\"Stock value\" format=ugx\n```\n\n```sheet name=\"Stock\" types=\"text,int,ugx,ugx\"\n| item | qty | price | total |\n|---|---|---|---|\n| Sugar | 40 | 4200 | =B2*C2 |\n```\n\nPrices as of this week."
        val sheet = NoteSheet("Notebooks/Stock.md", "Stock", "Notebooks", body, false, null)
        return home.copy(stack = listOf(NotesRoute.Reader(sheet.path)), reader = ReaderPhase.Ready(sheet, "edited just now"))
    }
    if (name?.startsWith("n-suite-") == true) {
        val source = "Notebooks/Inbox/Imports/Quarterly report.docx"
        val note = "---\ntitle: Quarterly report\nsource: $source\nsource_sha256: ${"ab".repeat(32)}\n---\n\n# Quarterly report\n\nImported from a Word document."
        val file = { state: String, headline: String, details: List<String>, shrink: Boolean, confirm: Boolean, undo: Boolean ->
            example.hq.spike.data.SuiteFile(
                source, state, if (shrink) null else "sj-751bd766cd684884", "Notebooks/Inbox/Imports/Quarterly report.md", 930652, headline, details,
                shrink, confirm, confirm, undo,
            )
        }
        SuiteBarFixture.state = when (name) {
            "n-suite-ready" -> file("ready", "Quarterly report.docx can be made smaller.", emptyList(), true, false, false) to null
            "n-suite-prepared" -> file(
                "prepared", "Quarterly report.docx can shrink from 3.4 MB to 2.5 MB.",
                listOf("Original 3.4 MB, record 2.5 MB: 909 KB saved.", "The note does not keep page layout (margins, fonts, columns).", "Your original has not been moved."),
                false, true, false,
            ) to null
            "n-suite-shrunk" -> file("shrunk", "Quarterly report.docx was made smaller. 909 KB saved; the original is in the trash for 30 days.", emptyList(), false, false, true) to null
            else -> file("ready", "", emptyList(), true, false, false) to "The note would not be smaller than the file, so there is nothing to save."
        }
        val sheet = NoteSheet("Notebooks/Inbox/Imports/Quarterly report.md", "Quarterly report", "Inbox › Imports", note, false, null)
        return home.copy(stack = listOf(NotesRoute.Reader(sheet.path)), reader = ReaderPhase.Ready(sheet, "edited just now"))
    }
    return when (name) {
        "n-real-reader" -> home.copy(stack = listOf(NotesRoute.Reader(longSheet.path)), reader = longReader)
        "n-home" -> home
        "n-empty" -> NotesUi(loaded = true)
        "n-offline" -> NotesUi(loaded = true, offline = true)
        "n-reader" -> home.copy(stack = listOf(NotesRoute.Reader(sheet.path)), reader = reader)
        "n-menu" -> home.copy(stack = listOf(NotesRoute.Reader(sheet.path)), reader = reader, menu = true)
        "n-gone" -> home.copy(stack = listOf(NotesRoute.Reader("Notebooks/Projects/Old-plan.md")), reader = ReaderPhase.Gone)
        "n-folder" -> home.copy(
            stack = listOf(NotesRoute.Folder("Notebooks/Projects/Agent-HQ")),
            folder = FolderView(
                "Notebooks/Projects/Agent-HQ", listOf("Notebooks/Projects/Agent-HQ/Android spikes" to 7, "Notebooks/Projects/Agent-HQ/Evidence" to 12),
                listOf("Notebooks/Projects/Agent-HQ/Mobile-First-Roadmap-2026-10-20.md", "Notebooks/Projects/Agent-HQ/Phone-Cutover-Evidence-2026-10-20.md"), loading = false,
            ),
        )
        "n-write" -> {
            val text = "Do these four things before the soak test starts.\n\n1. Open Settings, then Battery.\n2. Find HQ in the list of apps."
            home.copy(stack = listOf(NotesRoute.Write(null)), write = WriteState(null, INBOX, "Samsung battery steps", TextFieldValue(text, TextRange(text.length)), "", status = SaveStatus.Unsaved))
        }
        "n-edit" -> {
            val text = "## Workstream C\n\nA native view instead of the WebView.\n\n- **Chat** first, then approvals\n- [[Android-On-Device-Assessment]]"
            home.copy(stack = listOf(NotesRoute.Reader(sheet.path), NotesRoute.Write(sheet.path)), reader = reader, write = WriteState(sheet.path, "Projects › Agent-HQ", "Mobile-First-Roadmap", TextFieldValue(text, TextRange(text.length)), "x", status = SaveStatus.Unsaved))
        }
        "n-leave" -> {
            val text = "Draft"
            home.copy(stack = listOf(NotesRoute.Write(null)), write = WriteState(null, INBOX, "Samsung battery steps", TextFieldValue(text, TextRange(text.length)), "", leaving = true))
        }
        "n-chooser" -> home.copy(stack = listOf(NotesRoute.Write(null)), write = WriteState(null, INBOX, "Samsung battery steps", TextFieldValue(""), "", chooser = true))
        else -> null
    }
}

/** A note the size and shape of the project notes on the phone: a doubled frontmatter block, a long table with bold cells, many paragraphs. */
private fun longNote(): String = buildString {
    append("---\ntitle: ''\n---\n\n---\ntitle: Build-Plan-And-Cutover\ntags: [plan, android, cutover]\n---\n\n")
    append("# Native app: build order, design validation and cutover\n\nSources: the design canvas, the roadmap and the voice plan. Code: branch `feat/android-gradle`, `apps/android`.\n\n")
    append("## Where it stands\n|Slice|State|\n|---|---|\n")
    repeat(7) { i ->
        append("|S${i + 1} Gradle build and the first screens|**Done and live**, the pull request is open and not merged. The last install kept the sealed token and the credentials, and the vault database grew by the expected amount.|\n")
    }
    append("\n")
    repeat(14) { section ->
        append("## Section ${section + 1}\n\n")
        append("Method: a render of each board frame, the app on an emulator at 390 dp, and the app on the phone. Matching: name row, bubble shapes and padding, streaming headline and steps, the sheet with its header, search, group labels and rows. Differences are listed with their reasons.\n\n")
        append("- **Fixed** the quote bar height inside a list, the fades that never drew, the first column width.\n- Found only by using it: archiving the open chat closed the sheet five seconds later.\n- `Where it goes next` is written in the project notes.\n\n")
    }
}
