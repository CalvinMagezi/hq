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
    return when (name) {
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
