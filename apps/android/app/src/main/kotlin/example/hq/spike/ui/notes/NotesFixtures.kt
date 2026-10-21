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
    val hits = listOf(
        SearchHit("Notebooks/Projects/Agent-HQ/Mobile-First-Roadmap.md", "Mobile-First-Roadmap", "Projects", "…where AgentHQ goes from here. Workstream C: a native view… roadmap for the phone"),
        SearchHit("Notebooks/Projects/Agent-HQ/Native-Bridge-Plan.md", "Native-Bridge-Plan", "Projects", "The bridge between the roadmap and the app."),
        SearchHit("Notebooks/Inbox/Quick idea widget copy.md", "Quick idea: widget copy", "Inbox", "A roadmap item for later."),
    )
    val longSheet = NoteSheet("Notebooks/Projects/Agent-HQ/Build-Plan-And-Cutover.md", "Build-Plan-And-Cutover", "Projects › Agent-HQ", longNote(), false, null)
    val longReader = ReaderPhase.Ready(longSheet, "edited 33m ago")
    val realHits = (1..13).map { i ->
        SearchHit("Notebooks/Projects/Agent-HQ/Design-Note-$i.md", "Design-Note-$i", "Projects", if (i == 1) "…reuse what is already built\nThe tunnel hub (hq0, hq net, the root applier, ufw) stays on the current droplet and carries the traffic until the cutover…" else if (i % 2 == 0) "…1. **What it is.** A userspace tunnel that runs inside the app, configured by `hq net`, with [[Private-Network-Plan]] as the source…" else "…a plan for the tunnel, see [the design](Design.md) and the **first test round** results…")
    }
    return when (name) {
        "n-real-reader" -> home.copy(stack = listOf(NotesRoute.Reader(longSheet.path)), reader = longReader)
        "n-real-search" -> home.copy(stack = listOf(NotesRoute.Search), search = SearchState("tunnel", realHits))
        "n-home" -> home
        "n-empty" -> NotesUi(loaded = true)
        "n-offline" -> NotesUi(loaded = true, offline = true)
        "n-reader" -> home.copy(stack = listOf(NotesRoute.Reader(sheet.path)), reader = reader)
        "n-menu" -> home.copy(stack = listOf(NotesRoute.Reader(sheet.path)), reader = reader, menu = true)
        "n-gone" -> home.copy(stack = listOf(NotesRoute.Reader("Notebooks/Projects/Old-plan.md")), reader = ReaderPhase.Gone)
        "n-search" -> home.copy(stack = listOf(NotesRoute.Search), search = SearchState("roadmap", hits))
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
